// protocol/iterated_contract_net.rs - FIPA Iterated Contract Net Protocol
//
//! FIPA Iterated Contract Net Protocol Implementation
//!
//! An extension of the Contract Net protocol that allows multiple
//! rounds of negotiation. Useful when:
//! - Initial proposals are not satisfactory
//! - Requirements may change during negotiation
//! - Fine-tuning of proposals is needed
//!
//! # Protocol Flow
//!
//! ```text
//! Initiator                     Participants
//!     |                              |
//!     |---------- CFP -------------->|  (Round 1)
//!     |                              |
//!     |<--------- PROPOSE -----------|
//!     |                              |
//!     |-- REJECT-PROPOSAL (revised)->|  (New CFP for Round 2)
//!     |                              |
//!     |<--------- PROPOSE -----------|  (Revised proposals)
//!     |                              |
//!     |  ... repeat as needed ...    |
//!     |                              |
//!     |------ ACCEPT-PROPOSAL ------>|  (Final acceptance)
//!     |                              |
//!     |<--------- INFORM ------------|  (Result)
//!     |                              |
//! ```

use super::state_machine::*;
use super::contract_net::Proposal;
use crate::proto;
use std::collections::HashMap;

/// Round information
#[derive(Debug, Clone)]
pub struct NegotiationRound {
    /// Round number
    pub round: u32,
    /// CFP content for this round
    pub cfp_content: Vec<u8>,
    /// Proposals received
    pub proposals: Vec<Proposal>,
    /// Timestamp
    pub timestamp: i64,
}

/// Iterated Contract Net Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IteratedContractNetState {
    /// Initial state
    NotStarted,
    /// CFP sent, awaiting proposals
    CfpSent,
    /// Proposals received, evaluating
    Evaluating,
    /// Requesting revised proposals
    Revising,
    /// Final proposal accepted
    Accepted,
    /// Waiting for result
    WaitingResult,
    /// Completed successfully
    Completed,
    /// All proposals rejected / no acceptable proposal
    NoAgreement,
    /// Failed
    Failed,
    /// Cancelled
    Cancelled,
}

impl IteratedContractNetState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IteratedContractNetState::NotStarted => "not_started",
            IteratedContractNetState::CfpSent => "cfp_sent",
            IteratedContractNetState::Evaluating => "evaluating",
            IteratedContractNetState::Revising => "revising",
            IteratedContractNetState::Accepted => "accepted",
            IteratedContractNetState::WaitingResult => "waiting_result",
            IteratedContractNetState::Completed => "completed",
            IteratedContractNetState::NoAgreement => "no_agreement",
            IteratedContractNetState::Failed => "failed",
            IteratedContractNetState::Cancelled => "cancelled",
        }
    }
}

/// Iterated Contract Net Protocol Implementation
#[derive(Debug)]
pub struct IteratedContractNetProtocol {
    /// Current state
    state: IteratedContractNetState,

    /// Conversation base
    base: ConversationBase,

    /// Current round number
    current_round: u32,

    /// Maximum rounds allowed
    max_rounds: u32,

    /// Round history
    rounds: Vec<NegotiationRound>,

    /// Current proposals (for current round)
    current_proposals: HashMap<String, Proposal>,

    /// Selected contractor
    selected_contractor: Option<String>,

    /// Final result
    result: Option<Vec<u8>>,

    /// Deadline per round (milliseconds)
    round_deadline: i64,
}

impl IteratedContractNetProtocol {
    /// Create a new iterated contract net protocol (as initiator)
    pub fn new_as_initiator(max_rounds: u32) -> Self {
        Self {
            state: IteratedContractNetState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            current_round: 0,
            max_rounds,
            rounds: vec![],
            current_proposals: HashMap::new(),
            selected_contractor: None,
            result: None,
            round_deadline: 30000, // 30 seconds default
        }
    }

    /// Create a new iterated contract net protocol (as participant)
    pub fn new_as_participant() -> Self {
        Self {
            state: IteratedContractNetState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Participant),
            current_round: 0,
            max_rounds: 10, // Default, will be updated from messages
            rounds: vec![],
            current_proposals: HashMap::new(),
            selected_contractor: None,
            result: None,
            round_deadline: 30000,
        }
    }

    /// Set round deadline
    pub fn with_round_deadline(mut self, deadline_ms: i64) -> Self {
        self.round_deadline = deadline_ms;
        self
    }

    /// Set conversation ID
    pub fn with_conversation_id(mut self, id: String) -> Self {
        self.base.conversation_id = id;
        self
    }

    /// Get current round
    pub fn current_round(&self) -> u32 {
        self.current_round
    }

    /// Get max rounds
    pub fn max_rounds(&self) -> u32 {
        self.max_rounds
    }

    /// Get round history
    pub fn rounds(&self) -> &[NegotiationRound] {
        &self.rounds
    }

    /// Get current proposals
    pub fn current_proposals(&self) -> &HashMap<String, Proposal> {
        &self.current_proposals
    }

    /// Get selected contractor
    pub fn selected_contractor(&self) -> Option<&str> {
        self.selected_contractor.as_deref()
    }

    /// Can start another round?
    pub fn can_continue(&self) -> bool {
        self.current_round < self.max_rounds
    }

    /// Start a new round with updated CFP
    pub fn start_round(&mut self, cfp_content: Vec<u8>) -> Result<u32, ProtocolError> {
        if !self.can_continue() {
            return Err(ProtocolError::ValidationFailed("Maximum rounds reached".into()));
        }

        if !matches!(
            self.state,
            IteratedContractNetState::NotStarted | IteratedContractNetState::Evaluating | IteratedContractNetState::Revising
        ) {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "start_round".to_string(),
            });
        }

        // Save current round if any proposals exist
        if !self.current_proposals.is_empty() {
            self.rounds.push(NegotiationRound {
                round: self.current_round,
                cfp_content: cfp_content.clone(),
                proposals: self.current_proposals.values().cloned().collect(),
                timestamp: chrono::Utc::now().timestamp_millis(),
            });
        }

        self.current_round += 1;
        self.current_proposals.clear();
        self.state = IteratedContractNetState::CfpSent;

        Ok(self.current_round)
    }

    /// Add a proposal for current round
    pub fn add_proposal(&mut self, proposal: Proposal) -> Result<(), ProtocolError> {
        if self.state != IteratedContractNetState::CfpSent {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "add_proposal".to_string(),
            });
        }

        self.current_proposals.insert(proposal.bidder.name.clone(), proposal);
        Ok(())
    }

    /// Enter evaluation phase
    pub fn start_evaluation(&mut self) -> Result<(), ProtocolError> {
        if self.state != IteratedContractNetState::CfpSent {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "start_evaluation".to_string(),
            });
        }

        if self.current_proposals.is_empty() {
            if self.current_round >= self.max_rounds {
                self.state = IteratedContractNetState::NoAgreement;
            } else {
                self.state = IteratedContractNetState::Revising;
            }
        } else {
            self.state = IteratedContractNetState::Evaluating;
        }

        Ok(())
    }

    /// Request revision (another round)
    pub fn request_revision(&mut self) -> Result<(), ProtocolError> {
        if self.state != IteratedContractNetState::Evaluating {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "request_revision".to_string(),
            });
        }

        if !self.can_continue() {
            self.state = IteratedContractNetState::NoAgreement;
            return Err(ProtocolError::ValidationFailed("Maximum rounds reached".into()));
        }

        self.state = IteratedContractNetState::Revising;
        Ok(())
    }

    /// Accept a proposal
    pub fn accept_proposal(&mut self, proposer: &str) -> Result<&Proposal, ProtocolError> {
        if self.state != IteratedContractNetState::Evaluating {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "accept_proposal".to_string(),
            });
        }

        if !self.current_proposals.contains_key(proposer) {
            return Err(ProtocolError::ValidationFailed(format!(
                "No proposal from {}",
                proposer
            )));
        }

        self.selected_contractor = Some(proposer.to_string());
        self.state = IteratedContractNetState::Accepted;

        Ok(self.current_proposals.get(proposer).unwrap())
    }

    /// Record final result
    pub fn record_result(&mut self, result: Vec<u8>) -> Result<(), ProtocolError> {
        if !matches!(
            self.state,
            IteratedContractNetState::Accepted | IteratedContractNetState::WaitingResult
        ) {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "record_result".to_string(),
            });
        }

        self.result = Some(result);
        self.state = IteratedContractNetState::Completed;
        Ok(())
    }

    /// Validate state transition
    fn validate_transition(&self, performative: proto::Performative) -> Result<IteratedContractNetState, ProtocolError> {
        use proto::Performative::*;
        use IteratedContractNetState::*;

        match (&self.state, performative) {
            // Initial CFP
            (NotStarted, Cfp) => Ok(CfpSent),
            // Proposals
            (CfpSent, Propose) => Ok(CfpSent), // Still collecting
            (CfpSent, Refuse) => Ok(CfpSent), // Some may refuse
            // Accept or reject (which triggers new round)
            (CfpSent, AcceptProposal) | (Evaluating, AcceptProposal) => Ok(Accepted),
            (CfpSent, RejectProposal) | (Evaluating, RejectProposal) => {
                if self.can_continue() {
                    Ok(Revising)
                } else {
                    Ok(NoAgreement)
                }
            }
            // New CFP for revision
            (Revising, Cfp) => Ok(CfpSent),
            (Evaluating, Cfp) => Ok(CfpSent), // Revised CFP
            // Result
            (Accepted, InformDone) | (Accepted, InformResult) => Ok(Completed),
            (WaitingResult, InformDone) | (WaitingResult, InformResult) => Ok(Completed),
            // Failure
            (Accepted, Failure) | (WaitingResult, Failure) => Ok(Failed),
            // Cancel
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for IteratedContractNetProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolIteratedContractNet
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
                if self.state == IteratedContractNetState::NotStarted {
                    self.current_round = 1;
                } else {
                    self.current_round += 1;
                    self.current_proposals.clear();
                }
            }
            proto::Performative::Propose => {
                if let Some(sender) = &msg.sender {
                    let proposal = Proposal {
                        bidder: sender.clone(),
                        content: msg.content.clone(),
                        received_at: chrono::Utc::now().timestamp_millis(),
                    };
                    self.current_proposals.insert(sender.name.clone(), proposal);
                }
            }
            proto::Performative::AcceptProposal => {
                // For participant, this means they were selected
            }
            proto::Performative::InformDone | proto::Performative::InformResult => {
                self.result = Some(msg.content.clone());
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            IteratedContractNetState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: self.result.clone(),
                ..Default::default()
            })),
            IteratedContractNetState::NoAgreement => {
                Ok(ProcessResult::Failed("No agreement reached".into()))
            }
            IteratedContractNetState::Failed => Ok(ProcessResult::Failed("Contract failed".into())),
            IteratedContractNetState::Cancelled => {
                Ok(ProcessResult::Failed("Contract cancelled".into()))
            }
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            IteratedContractNetState::Completed
                | IteratedContractNetState::NoAgreement
                | IteratedContractNetState::Failed
                | IteratedContractNetState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            IteratedContractNetState::NoAgreement
                | IteratedContractNetState::Failed
                | IteratedContractNetState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            IteratedContractNetState::NotStarted => vec![Cfp],
            IteratedContractNetState::CfpSent => {
                vec![Propose, Refuse, AcceptProposal, RejectProposal, Cancel]
            }
            IteratedContractNetState::Evaluating => {
                vec![AcceptProposal, RejectProposal, Cfp, Cancel]
            }
            IteratedContractNetState::Revising => vec![Cfp, Cancel],
            IteratedContractNetState::Accepted | IteratedContractNetState::WaitingResult => {
                vec![InformDone, InformResult, Failure, Cancel]
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iterated_contract_net_basics() {
        let protocol = IteratedContractNetProtocol::new_as_initiator(5);
        assert_eq!(protocol.state, IteratedContractNetState::NotStarted);
        assert_eq!(protocol.max_rounds(), 5);
        assert!(protocol.can_continue());
    }

    #[test]
    fn test_multiple_rounds() {
        let mut protocol = IteratedContractNetProtocol::new_as_initiator(3);

        // Round 1
        protocol.start_round(b"cfp1".to_vec()).unwrap();
        assert_eq!(protocol.current_round(), 1);

        protocol.add_proposal(Proposal {
            bidder: proto::AgentId {
                name: "agent1".into(),
                addresses: vec![],
                resolvers: vec![],
            },
            content: b"proposal1".to_vec(),
            received_at: 0,
        }).unwrap();

        protocol.start_evaluation().unwrap();
        protocol.request_revision().unwrap();

        // Round 2
        protocol.start_round(b"cfp2".to_vec()).unwrap();
        assert_eq!(protocol.current_round(), 2);
    }

    #[test]
    fn test_accept_proposal() {
        let mut protocol = IteratedContractNetProtocol::new_as_initiator(3);

        protocol.start_round(b"cfp".to_vec()).unwrap();
        protocol.add_proposal(Proposal {
            bidder: proto::AgentId {
                name: "agent1".into(),
                addresses: vec![],
                resolvers: vec![],
            },
            content: b"proposal".to_vec(),
            received_at: 0,
        }).unwrap();

        protocol.start_evaluation().unwrap();
        let accepted = protocol.accept_proposal("agent1").unwrap();

        assert_eq!(accepted.bidder.name, "agent1");
        assert_eq!(protocol.selected_contractor(), Some("agent1"));
    }
}
