// protocol/contract_net.rs - FIPA Contract Net Protocol

use super::state_machine::*;
use crate::proto;
use std::collections::HashMap;

/// FIPA Contract Net Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractNetState {
    NotStarted,
    CfpSent,
    ProposalsReceived,
    Evaluating,
    Accepted,
    Rejected,
    InExecution,
    Completed,
    Failed,
}

impl ContractNetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContractNetState::NotStarted => "not_started",
            ContractNetState::CfpSent => "cfp_sent",
            ContractNetState::ProposalsReceived => "proposals_received",
            ContractNetState::Evaluating => "evaluating",
            ContractNetState::Accepted => "accepted",
            ContractNetState::Rejected => "rejected",
            ContractNetState::InExecution => "in_execution",
            ContractNetState::Completed => "completed",
            ContractNetState::Failed => "failed",
        }
    }
}

/// Proposal from a contractor
#[derive(Debug, Clone)]
pub struct Proposal {
    pub bidder: proto::AgentId,
    pub content: Vec<u8>,
    pub received_at: i64,
}

/// FIPA Contract Net Protocol Implementation
#[derive(Debug)]
pub struct ContractNetProtocol {
    state: ContractNetState,
    base: ConversationBase,

    /// Task description from CFP
    task_description: Option<Vec<u8>>,

    /// Expected number of participants
    expected_participants: usize,

    /// Received proposals
    proposals: Vec<Proposal>,

    /// Accepted proposals (bidder name -> proposal)
    accepted: HashMap<String, Proposal>,

    /// Results from accepted contractors
    results: HashMap<String, Vec<u8>>,
}

impl ContractNetProtocol {
    pub fn new(role: Role) -> Self {
        Self {
            state: ContractNetState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), role),
            task_description: None,
            expected_participants: 0,
            proposals: Vec::new(),
            accepted: HashMap::new(),
            results: HashMap::new(),
        }
    }

    /// Set expected number of participants
    pub fn with_expected_participants(mut self, count: usize) -> Self {
        self.expected_participants = count;
        self
    }

    /// Get all proposals
    pub fn proposals(&self) -> &[Proposal] {
        &self.proposals
    }

    /// Accept a proposal
    pub fn accept_proposal(&mut self, bidder_name: &str) -> Option<Proposal> {
        self.proposals
            .iter()
            .find(|p| p.bidder.name == bidder_name)
            .cloned()
            .map(|p| {
                self.accepted.insert(bidder_name.to_string(), p.clone());
                p
            })
    }

    fn validate_transition(&self, performative: proto::Performative) -> Result<ContractNetState, ProtocolError> {
        use proto::Performative::*;
        use ContractNetState::*;

        match (&self.state, performative) {
            (NotStarted, Cfp) => Ok(CfpSent),
            (CfpSent, Propose) => Ok(ProposalsReceived),
            (CfpSent, Refuse) => Ok(ProposalsReceived),
            (ProposalsReceived, Propose) => Ok(ProposalsReceived),
            (ProposalsReceived, Refuse) => Ok(ProposalsReceived),
            (ProposalsReceived, AcceptProposal) => Ok(InExecution),
            (ProposalsReceived, RejectProposal) => Ok(Rejected),
            (InExecution, InformDone) | (InExecution, InformResult) => Ok(Completed),
            (InExecution, Failure) => Ok(Failed),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for ContractNetProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolContractNet
    }

    fn state_name(&self) -> &str {
        self.state.as_str()
    }

    fn validate(&self, msg: &proto::AclMessage) -> Result<(), ProtocolError> {
        let performative = proto::Performative::try_from(msg.performative)
            .map_err(|_| ProtocolError::ValidationFailed("Invalid performative".into()))?;
        self.validate_transition(performative)?;
        Ok(())
    }

    fn process(&mut self, msg: proto::AclMessage) -> Result<ProcessResult, ProtocolError> {
        let performative = proto::Performative::try_from(msg.performative)
            .map_err(|_| ProtocolError::ValidationFailed("Invalid performative".into()))?;

        let new_state = self.validate_transition(performative)?;

        self.base.record_message(msg.clone());

        match performative {
            proto::Performative::Cfp => {
                self.task_description = Some(msg.content.clone());
            }
            proto::Performative::Propose => {
                if let Some(sender) = &msg.sender {
                    self.proposals.push(Proposal {
                        bidder: sender.clone(),
                        content: msg.content.clone(),
                        received_at: chrono::Utc::now().timestamp_millis(),
                    });
                    self.base.add_participant(sender.clone());
                }
            }
            proto::Performative::InformDone | proto::Performative::InformResult => {
                if let Some(sender) = &msg.sender {
                    self.results.insert(sender.name.clone(), msg.content.clone());
                }
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            ContractNetState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: self.results.values().next().cloned(),
                ..Default::default()
            })),
            ContractNetState::Failed | ContractNetState::Rejected => {
                Ok(ProcessResult::Failed("Contract net failed".into()))
            }
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            ContractNetState::Completed | ContractNetState::Failed | ContractNetState::Rejected
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            ContractNetState::Failed | ContractNetState::Rejected
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            ContractNetState::NotStarted => vec![Cfp],
            ContractNetState::CfpSent => vec![Propose, Refuse],
            ContractNetState::ProposalsReceived => {
                vec![Propose, Refuse, AcceptProposal, RejectProposal]
            }
            ContractNetState::InExecution => vec![InformDone, InformResult, Failure],
            _ => vec![],
        }
    }

    fn serialize_state(&self) -> Result<Vec<u8>, ProtocolError> {
        Ok(self.state.as_str().as_bytes().to_vec())
    }

    fn message_history(&self) -> &[proto::AclMessage] {
        &self.base.messages
    }
}
