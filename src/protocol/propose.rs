// protocol/propose.rs - FIPA Propose Protocol
//
//! FIPA Propose Protocol Implementation
//!
//! The Propose protocol allows an initiator to propose an action to a participant,
//! who can then accept or reject the proposal.
//!
//! # Protocol Flow
//!
//! ```text
//! Initiator                     Participant
//!     |                              |
//!     |--------- PROPOSE ----------->|
//!     |                              |
//!     |<-------- ACCEPT-PROPOSAL ----|
//!     |   or                         |
//!     |<-------- REJECT-PROPOSAL ----|
//!     |                              |
//! ```

use super::state_machine::*;
use crate::proto;

/// FIPA Propose Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeState {
    /// Initial state
    NotStarted,
    /// Proposal has been sent
    Proposed,
    /// Proposal was accepted
    Accepted,
    /// Proposal was rejected
    Rejected,
    /// Protocol was cancelled
    Cancelled,
}

impl ProposeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposeState::NotStarted => "not_started",
            ProposeState::Proposed => "proposed",
            ProposeState::Accepted => "accepted",
            ProposeState::Rejected => "rejected",
            ProposeState::Cancelled => "cancelled",
        }
    }
}

/// FIPA Propose Protocol Implementation
#[derive(Debug)]
pub struct ProposeProtocol {
    /// Current state
    state: ProposeState,

    /// Conversation base
    base: ConversationBase,

    /// Proposal content
    proposal_content: Option<Vec<u8>>,

    /// Response content
    response_content: Option<Vec<u8>>,
}

impl ProposeProtocol {
    /// Create a new propose protocol instance
    pub fn new(role: Role) -> Self {
        Self {
            state: ProposeState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), role),
            proposal_content: None,
            response_content: None,
        }
    }

    /// Create with a specific conversation ID
    pub fn with_conversation_id(mut self, id: String) -> Self {
        self.base.conversation_id = id;
        self
    }

    /// Set deadline
    pub fn with_deadline(mut self, deadline: i64) -> Self {
        self.base.deadline = Some(deadline);
        self
    }

    /// Validate state transition
    fn validate_transition(&self, performative: proto::Performative) -> Result<ProposeState, ProtocolError> {
        use proto::Performative::*;
        use ProposeState::*;

        match (&self.state, performative) {
            (NotStarted, Propose) => Ok(Proposed),
            (Proposed, AcceptProposal) => Ok(Accepted),
            (Proposed, RejectProposal) => Ok(Rejected),
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }

    /// Get proposal content
    pub fn proposal(&self) -> Option<&[u8]> {
        self.proposal_content.as_deref()
    }

    /// Get response content
    pub fn response(&self) -> Option<&[u8]> {
        self.response_content.as_deref()
    }
}

impl ProtocolStateMachine for ProposeProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolPropose
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

        match &new_state {
            ProposeState::Proposed => {
                self.proposal_content = Some(msg.content.clone());
                if let Some(sender) = &msg.sender {
                    self.base.add_participant(sender.clone());
                }
            }
            ProposeState::Accepted | ProposeState::Rejected => {
                self.response_content = Some(msg.content.clone());
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            ProposeState::Proposed => Ok(ProcessResult::Continue),
            ProposeState::Accepted => Ok(ProcessResult::Complete(CompletionData {
                result: self.response_content.clone(),
                ..Default::default()
            })),
            ProposeState::Rejected => Ok(ProcessResult::Failed("Proposal rejected".into())),
            ProposeState::Cancelled => Ok(ProcessResult::Failed("Protocol cancelled".into())),
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            ProposeState::Accepted | ProposeState::Rejected | ProposeState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            ProposeState::Rejected | ProposeState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            ProposeState::NotStarted => vec![Propose],
            ProposeState::Proposed => vec![AcceptProposal, RejectProposal, Cancel],
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

    fn create_test_message(performative: proto::Performative) -> proto::AclMessage {
        proto::AclMessage {
            message_id: "msg-1".into(),
            performative: performative as i32,
            sender: Some(proto::AgentId {
                name: "sender".into(),
                addresses: vec![],
                resolvers: vec![],
            }),
            receivers: vec![proto::AgentId {
                name: "receiver".into(),
                addresses: vec![],
                resolvers: vec![],
            }],
            content: b"test content".to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn test_propose_accept_flow() {
        let mut protocol = ProposeProtocol::new(Role::Participant);
        assert_eq!(protocol.state, ProposeState::NotStarted);

        let propose = create_test_message(proto::Performative::Propose);
        let result = protocol.process(propose).unwrap();
        assert!(matches!(result, ProcessResult::Continue));
        assert_eq!(protocol.state, ProposeState::Proposed);

        let accept = create_test_message(proto::Performative::AcceptProposal);
        let result = protocol.process(accept).unwrap();
        assert!(matches!(result, ProcessResult::Complete(_)));
        assert_eq!(protocol.state, ProposeState::Accepted);
    }

    #[test]
    fn test_propose_reject_flow() {
        let mut protocol = ProposeProtocol::new(Role::Participant);

        let propose = create_test_message(proto::Performative::Propose);
        protocol.process(propose).unwrap();

        let reject = create_test_message(proto::Performative::RejectProposal);
        let result = protocol.process(reject).unwrap();
        assert!(matches!(result, ProcessResult::Failed(_)));
        assert!(protocol.is_failed());
    }
}
