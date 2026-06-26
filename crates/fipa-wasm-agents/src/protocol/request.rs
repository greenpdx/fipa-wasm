// protocol/request.rs - FIPA Request Protocol

use super::state_machine::*;
use crate::proto;

/// FIPA Request Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    /// Initial state
    NotStarted,
    /// Request has been sent
    Requested,
    /// Participant agreed to perform action
    Agreed,
    /// Action is being executed
    Executing,
    /// Successfully completed
    Completed,
    /// Action failed
    Failed,
    /// Request was refused
    Refused,
    /// Protocol was cancelled
    Cancelled,
}

impl RequestState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestState::NotStarted => "not_started",
            RequestState::Requested => "requested",
            RequestState::Agreed => "agreed",
            RequestState::Executing => "executing",
            RequestState::Completed => "completed",
            RequestState::Failed => "failed",
            RequestState::Refused => "refused",
            RequestState::Cancelled => "cancelled",
        }
    }
}

/// FIPA Request Protocol Implementation
#[derive(Debug)]
pub struct RequestProtocol {
    /// Current state
    state: RequestState,

    /// Conversation base
    base: ConversationBase,

    /// Request content (for reference)
    request_content: Option<Vec<u8>>,

    /// Result content
    result: Option<Vec<u8>>,
}

impl RequestProtocol {
    /// Create a new request protocol instance
    pub fn new(role: Role) -> Self {
        Self {
            state: RequestState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), role),
            request_content: None,
            result: None,
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
    fn validate_transition(&self, performative: proto::Performative) -> Result<RequestState, ProtocolError> {
        use proto::Performative::*;
        use RequestState::*;

        match (&self.state, performative) {
            (NotStarted, Request) => Ok(Requested),
            (Requested, Agree) => Ok(Agreed),
            (Requested, Refuse) => Ok(Refused),
            (Agreed, InformDone) => Ok(Completed),
            (Agreed, InformResult) => Ok(Completed),
            (Agreed, Failure) => Ok(Failed),
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for RequestProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolRequest
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

        // Validate and get new state
        let new_state = self.validate_transition(performative)?;

        // Record message
        self.base.record_message(msg.clone());

        // Handle state-specific logic
        match &new_state {
            RequestState::Requested => {
                self.request_content = Some(msg.content.clone());
                if let Some(sender) = &msg.sender {
                    self.base.add_participant(sender.clone());
                }
            }
            RequestState::Completed => {
                self.result = Some(msg.content.clone());
            }
            RequestState::Failed => {
                self.result = Some(msg.content.clone());
            }
            _ => {}
        }

        self.state = new_state;

        // Determine response based on role and state
        match (&self.base.role, &self.state) {
            (Role::Participant, RequestState::Requested) => {
                // Participant should respond with agree/refuse
                // For now, return Continue and let the agent decide
                Ok(ProcessResult::Continue)
            }
            (_, RequestState::Completed) => {
                Ok(ProcessResult::Complete(CompletionData {
                    result: self.result.clone(),
                    ..Default::default()
                }))
            }
            (_, RequestState::Failed) => {
                Ok(ProcessResult::Failed("Request failed".into()))
            }
            (_, RequestState::Refused) => {
                Ok(ProcessResult::Failed("Request refused".into()))
            }
            (_, RequestState::Cancelled) => {
                Ok(ProcessResult::Failed("Request cancelled".into()))
            }
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            RequestState::Completed
                | RequestState::Failed
                | RequestState::Refused
                | RequestState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            RequestState::Failed | RequestState::Refused | RequestState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            RequestState::NotStarted => vec![Request],
            RequestState::Requested => vec![Agree, Refuse, Cancel],
            RequestState::Agreed => vec![InformDone, InformResult, Failure, Cancel],
            _ => vec![],
        }
    }

    fn serialize_state(&self) -> Result<Vec<u8>, ProtocolError> {
        // Simple state serialization
        let state_str = self.state.as_str();
        Ok(state_str.as_bytes().to_vec())
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
    fn test_request_protocol_flow() {
        let mut protocol = RequestProtocol::new(Role::Participant);
        assert_eq!(protocol.state, RequestState::NotStarted);

        // Receive request
        let request = create_test_message(proto::Performative::Request);
        let result = protocol.process(request).unwrap();
        assert!(matches!(result, ProcessResult::Continue));
        assert_eq!(protocol.state, RequestState::Requested);

        // Send agree
        let agree = create_test_message(proto::Performative::Agree);
        let result = protocol.process(agree).unwrap();
        assert!(matches!(result, ProcessResult::Continue));
        assert_eq!(protocol.state, RequestState::Agreed);

        // Send inform-done
        let done = create_test_message(proto::Performative::InformDone);
        let result = protocol.process(done).unwrap();
        assert!(matches!(result, ProcessResult::Complete(_)));
        assert_eq!(protocol.state, RequestState::Completed);
        assert!(protocol.is_complete());
    }

    #[test]
    fn test_request_refused() {
        let mut protocol = RequestProtocol::new(Role::Participant);

        let request = create_test_message(proto::Performative::Request);
        protocol.process(request).unwrap();

        let refuse = create_test_message(proto::Performative::Refuse);
        let result = protocol.process(refuse).unwrap();

        assert!(matches!(result, ProcessResult::Failed(_)));
        assert!(protocol.is_complete());
        assert!(protocol.is_failed());
    }
}
