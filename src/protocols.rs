// protocols.rs
// FIPA protocol implementations with state machines

use crate::acl_message::*;
use std::collections::HashMap;
use std::fmt::Debug;

/// Protocol error types
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Invalid state transition")]
    InvalidTransition,
    #[error("Message validation failed: {0}")]
    ValidationFailed(String),
    #[error("Protocol not supported")]
    NotSupported,
    #[error("Missing conversation ID")]
    MissingConversationId,
    #[error("Unknown conversation")]
    UnknownConversation,
}

/// Generic protocol trait - all protocols must implement this
pub trait Protocol: Send + Sync {
    type State: Clone + Debug + Send + Sync;
    type Metadata: Clone + Debug + Send + Sync;

    fn validate_message(
        &self,
        msg: &AclMessage,
        current_state: &Self::State,
    ) -> Result<(), ProtocolError>;

    fn transition(
        &self,
        from: Self::State,
        msg: &AclMessage,
    ) -> Result<Self::State, ProtocolError>;

    fn is_terminal(&self, state: &Self::State) -> bool;
}

// ============================================================================
// Request Protocol
// ============================================================================

pub struct RequestProtocol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestState {
    NotStarted,
    Requested,
    Agreed,
    Executing,
    Completed,
    Failed,
    Refused,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct RequestMetadata {
    pub start_time: Timestamp,
    pub request_details: String,
    pub deadline: Option<Timestamp>,
}

impl Protocol for RequestProtocol {
    type State = RequestState;
    type Metadata = RequestMetadata;

    fn validate_message(
        &self,
        msg: &AclMessage,
        current_state: &Self::State,
    ) -> Result<(), ProtocolError> {
        use Performative::*;
        use RequestState::*;

        match (current_state, msg.header.performative) {
            (NotStarted, Request) => Ok(()),
            (Requested, Refuse | Agree) => Ok(()),
            (Agreed, InformDone | InformResult | Failure) => Ok(()),
            (_, Cancel) => Ok(()),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn transition(
        &self,
        from: Self::State,
        msg: &AclMessage,
    ) -> Result<Self::State, ProtocolError> {
        use Performative::*;
        use RequestState::*;

        match (from, msg.header.performative) {
            (NotStarted, Request) => Ok(Requested),
            (Requested, Agree) => Ok(Agreed),
            (Requested, Refuse) => Ok(Refused),
            (Agreed, InformDone | InformResult) => Ok(Completed),
            (Agreed, Failure) => Ok(Failed),
            (_, Cancel) => Ok(Cancelled),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(
            state,
            RequestState::Completed
                | RequestState::Failed
                | RequestState::Refused
                | RequestState::Cancelled
        )
    }
}

// ============================================================================
// Query Protocol
// ============================================================================

pub struct QueryProtocol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryState {
    NotStarted,
    Queried,
    Agreed,
    Completed,
    Failed,
    Refused,
}

#[derive(Clone, Debug)]
pub struct QueryMetadata {
    pub query_type: QueryType,
    pub query_content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryType {
    QueryIf,   // Boolean query
    QueryRef,  // Reference query
}

impl Protocol for QueryProtocol {
    type State = QueryState;
    type Metadata = QueryMetadata;

    fn validate_message(
        &self,
        msg: &AclMessage,
        current_state: &Self::State,
    ) -> Result<(), ProtocolError> {
        use Performative::*;
        use QueryState::*;

        match (current_state, msg.header.performative) {
            (NotStarted, QueryIf | QueryRef) => Ok(()),
            (Queried, Refuse | Agree) => Ok(()),
            (Agreed, InformIf | InformRef | InformResult | Failure) => Ok(()),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn transition(
        &self,
        from: Self::State,
        msg: &AclMessage,
    ) -> Result<Self::State, ProtocolError> {
        use Performative::*;
        use QueryState::*;

        match (from, msg.header.performative) {
            (NotStarted, QueryIf | QueryRef) => Ok(Queried),
            (Queried, Agree) => Ok(Agreed),
            (Queried, Refuse) => Ok(Refused),
            (Agreed, InformIf | InformRef | InformResult) => Ok(Completed),
            (Agreed, Failure) => Ok(Failed),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(
            state,
            QueryState::Completed | QueryState::Failed | QueryState::Refused
        )
    }
}

// ============================================================================
// Contract Net Protocol
// ============================================================================

pub struct ContractNetProtocol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractNetState {
    NotStarted,
    CfpSent,
    ProposalsReceived,
    Evaluating,
    Accepted(Vec<String>), // Agent names that were accepted
    Rejected(Vec<String>),
    InExecution,
    Completed,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ContractNetMetadata {
    pub task_description: String,
    pub deadline: Timestamp,
    pub expected_participants: usize,
    pub proposals: Vec<Proposal>,
    pub optimization_criteria: OptimizationCriteria,
}

#[derive(Clone, Debug)]
pub struct Proposal {
    pub bidder: AgentId,
    pub price: Option<f64>,
    pub completion_time: Option<Timestamp>,
    pub quality_metrics: HashMap<String, f64>,
    pub proposal_data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum OptimizationCriteria {
    LowestPrice,
    FastestCompletion,
    HighestQuality,
    Custom,
}

impl Protocol for ContractNetProtocol {
    type State = ContractNetState;
    type Metadata = ContractNetMetadata;

    fn validate_message(
        &self,
        msg: &AclMessage,
        current_state: &Self::State,
    ) -> Result<(), ProtocolError> {
        use ContractNetState::*;
        use Performative::*;

        match (current_state, msg.header.performative) {
            (NotStarted, Cfp) => Ok(()),
            (CfpSent, Propose | Refuse) => Ok(()),
            (ProposalsReceived, AcceptProposal | RejectProposal) => Ok(()),
            (InExecution, InformDone | InformResult | Failure) => Ok(()),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn transition(
        &self,
        from: Self::State,
        msg: &AclMessage,
    ) -> Result<Self::State, ProtocolError> {
        use ContractNetState::*;
        use Performative::*;

        match (from, msg.header.performative) {
            (NotStarted, Cfp) => Ok(CfpSent),
            (CfpSent, Propose) => Ok(ProposalsReceived),
            (CfpSent, Refuse) => Ok(ProposalsReceived),
            (ProposalsReceived, AcceptProposal) => Ok(InExecution),
            (ProposalsReceived, RejectProposal) => Ok(Rejected(vec![])),
            (InExecution, InformDone | InformResult) => Ok(Completed),
            (InExecution, Failure) => Ok(Failed),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(state, ContractNetState::Completed | ContractNetState::Failed)
    }
}

// ============================================================================
// Subscribe Protocol
// ============================================================================

pub struct SubscribeProtocol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscribeState {
    NotStarted,
    Subscribed,
    Agreed,
    Active,
    Completed,
    Failed,
    Refused,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct SubscribeMetadata {
    pub subscription_object: String,
    pub notification_count: usize,
}

impl Protocol for SubscribeProtocol {
    type State = SubscribeState;
    type Metadata = SubscribeMetadata;

    fn validate_message(
        &self,
        msg: &AclMessage,
        current_state: &Self::State,
    ) -> Result<(), ProtocolError> {
        use Performative::*;
        use SubscribeState::*;

        match (current_state, msg.header.performative) {
            (NotStarted, Subscribe) => Ok(()),
            (Subscribed, Refuse | Agree) => Ok(()),
            (Agreed, InformResult) => Ok(()),
            (Active, InformResult | Failure) => Ok(()),
            (_, Cancel) => Ok(()),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn transition(
        &self,
        from: Self::State,
        msg: &AclMessage,
    ) -> Result<Self::State, ProtocolError> {
        use Performative::*;
        use SubscribeState::*;

        match (from, msg.header.performative) {
            (NotStarted, Subscribe) => Ok(Subscribed),
            (Subscribed, Agree) => Ok(Agreed),
            (Subscribed, Refuse) => Ok(Refused),
            (Agreed, InformResult) => Ok(Active),
            (Active, InformResult) => Ok(Active), // Stay active
            (Active, Failure) => Ok(Failed),
            (_, Cancel) => Ok(Cancelled),
            _ => Err(ProtocolError::InvalidTransition),
        }
    }

    fn is_terminal(&self, state: &Self::State) -> bool {
        matches!(
            state,
            SubscribeState::Completed
                | SubscribeState::Failed
                | SubscribeState::Refused
                | SubscribeState::Cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_protocol_flow() {
        let protocol = RequestProtocol;
        let mut state = RequestState::NotStarted;

        // Request message
        let request_msg = AclMessage::new(
            Performative::Request,
            AgentId::new("agent1"),
            ReceiverSet::Single(AgentId::new("agent2")),
        );

        assert!(protocol.validate_message(&request_msg, &state).is_ok());
        state = protocol.transition(state, &request_msg).unwrap();
        assert_eq!(state, RequestState::Requested);

        // Agree message
        let agree_msg = AclMessage::new(
            Performative::Agree,
            AgentId::new("agent2"),
            ReceiverSet::Single(AgentId::new("agent1")),
        );

        state = protocol.transition(state, &agree_msg).unwrap();
        assert_eq!(state, RequestState::Agreed);

        // Complete
        let done_msg = AclMessage::new(
            Performative::InformDone,
            AgentId::new("agent2"),
            ReceiverSet::Single(AgentId::new("agent1")),
        );

        state = protocol.transition(state, &done_msg).unwrap();
        assert_eq!(state, RequestState::Completed);
        assert!(protocol.is_terminal(&state));
    }

    #[test]
    fn test_contract_net_basic_flow() {
        let protocol = ContractNetProtocol;
        let mut state = ContractNetState::NotStarted;

        // CFP
        let cfp = AclMessage::new(
            Performative::Cfp,
            AgentId::new("manager"),
            ReceiverSet::Multiple(vec![
                AgentId::new("worker1"),
                AgentId::new("worker2"),
            ]),
        );

        state = protocol.transition(state, &cfp).unwrap();
        assert_eq!(state, ContractNetState::CfpSent);

        // Proposal
        let proposal = AclMessage::new(
            Performative::Propose,
            AgentId::new("worker1"),
            ReceiverSet::Single(AgentId::new("manager")),
        );

        state = protocol.transition(state, &proposal).unwrap();
        assert_eq!(state, ContractNetState::ProposalsReceived);
    }
}
