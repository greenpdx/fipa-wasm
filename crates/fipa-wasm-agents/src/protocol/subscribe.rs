// protocol/subscribe.rs - FIPA Subscribe Protocol

use super::state_machine::*;
use crate::proto;

/// FIPA Subscribe Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl SubscribeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubscribeState::NotStarted => "not_started",
            SubscribeState::Subscribed => "subscribed",
            SubscribeState::Agreed => "agreed",
            SubscribeState::Active => "active",
            SubscribeState::Completed => "completed",
            SubscribeState::Failed => "failed",
            SubscribeState::Refused => "refused",
            SubscribeState::Cancelled => "cancelled",
        }
    }
}

/// FIPA Subscribe Protocol Implementation
#[derive(Debug)]
pub struct SubscribeProtocol {
    state: SubscribeState,
    base: ConversationBase,

    /// Subscription topic/object
    subscription_object: Option<Vec<u8>>,

    /// Number of notifications received
    notification_count: usize,

    /// Last notification content
    last_notification: Option<Vec<u8>>,
}

impl SubscribeProtocol {
    pub fn new(role: Role) -> Self {
        Self {
            state: SubscribeState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), role),
            subscription_object: None,
            notification_count: 0,
            last_notification: None,
        }
    }

    /// Get notification count
    pub fn notification_count(&self) -> usize {
        self.notification_count
    }

    fn validate_transition(&self, performative: proto::Performative) -> Result<SubscribeState, ProtocolError> {
        use proto::Performative::*;
        use SubscribeState::*;

        match (&self.state, performative) {
            (NotStarted, Subscribe) => Ok(Subscribed),
            (Subscribed, Agree) => Ok(Agreed),
            (Subscribed, Refuse) => Ok(Refused),
            (Agreed, InformResult) => Ok(Active),
            (Active, InformResult) => Ok(Active), // Stay active on notifications
            (Active, Failure) => Ok(Failed),
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for SubscribeProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolSubscribe
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
            proto::Performative::Subscribe => {
                self.subscription_object = Some(msg.content.clone());
                if let Some(sender) = &msg.sender {
                    self.base.add_participant(sender.clone());
                }
            }
            proto::Performative::InformResult => {
                self.notification_count += 1;
                self.last_notification = Some(msg.content.clone());
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            SubscribeState::Active => {
                // Return notification to agent
                Ok(ProcessResult::Continue)
            }
            SubscribeState::Completed => Ok(ProcessResult::Complete(CompletionData::default())),
            SubscribeState::Failed | SubscribeState::Refused | SubscribeState::Cancelled => {
                Ok(ProcessResult::Failed("Subscription ended".into()))
            }
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            SubscribeState::Completed
                | SubscribeState::Failed
                | SubscribeState::Refused
                | SubscribeState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            SubscribeState::Failed | SubscribeState::Refused | SubscribeState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            SubscribeState::NotStarted => vec![Subscribe],
            SubscribeState::Subscribed => vec![Agree, Refuse, Cancel],
            SubscribeState::Agreed | SubscribeState::Active => {
                vec![InformResult, Failure, Cancel]
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
