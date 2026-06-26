// protocol/state_machine.rs - Generic Protocol State Machine Trait

use crate::proto;
use std::fmt::Debug;

/// Protocol error types
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProtocolError {
    #[error("Invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Message validation failed: {0}")]
    ValidationFailed(String),

    #[error("Protocol not supported: {0:?}")]
    NotSupported(proto::ProtocolType),

    #[error("Missing conversation ID")]
    MissingConversationId,

    #[error("Unknown conversation: {0}")]
    UnknownConversation(String),

    #[error("Timeout waiting for response")]
    Timeout,

    #[error("Protocol cancelled")]
    Cancelled,

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// Result of processing a message in a protocol
#[derive(Debug, Clone)]
pub enum ProcessResult {
    /// Continue waiting for more messages
    Continue,

    /// Send a response message
    Respond(proto::AclMessage),

    /// Protocol completed successfully
    Complete(CompletionData),

    /// Protocol failed
    Failed(String),
}

/// Data returned when protocol completes
#[derive(Debug, Clone)]
pub struct CompletionData {
    /// Final result/outcome
    pub result: Option<Vec<u8>>,

    /// Metadata about completion
    pub metadata: std::collections::HashMap<String, String>,
}

impl Default for CompletionData {
    fn default() -> Self {
        Self {
            result: None,
            metadata: std::collections::HashMap::new(),
        }
    }
}

/// Generic protocol state machine trait
///
/// All FIPA protocols implement this trait to provide
/// type-safe state transitions and message validation.
pub trait ProtocolStateMachine: Send + Sync + Debug {
    /// Get the protocol type
    fn protocol_type(&self) -> proto::ProtocolType;

    /// Get current state name (for debugging/logging)
    fn state_name(&self) -> &str;

    /// Validate an incoming message against current state
    fn validate(&self, msg: &proto::AclMessage) -> Result<(), ProtocolError>;

    /// Process a message and transition state
    fn process(&mut self, msg: proto::AclMessage) -> Result<ProcessResult, ProtocolError>;

    /// Check if protocol is in a terminal state
    fn is_complete(&self) -> bool;

    /// Check if protocol has failed
    fn is_failed(&self) -> bool;

    /// Get the expected next performatives
    fn expected_performatives(&self) -> Vec<proto::Performative>;

    /// Serialize state for migration
    fn serialize_state(&self) -> Result<Vec<u8>, ProtocolError>;

    /// Get message history
    fn message_history(&self) -> &[proto::AclMessage];
}

/// Role in a protocol conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Initiated the protocol
    Initiator,
    /// Responding to initiator
    Participant,
    /// Mediating between parties
    Broker,
}

/// Base protocol conversation state
#[derive(Debug, Clone)]
pub struct ConversationBase {
    /// Conversation ID
    pub conversation_id: String,

    /// Our role in the conversation
    pub role: Role,

    /// Other participants
    pub participants: Vec<proto::AgentId>,

    /// Message history
    pub messages: Vec<proto::AclMessage>,

    /// Start timestamp
    pub start_time: i64,

    /// Deadline for completion (optional)
    pub deadline: Option<i64>,
}

impl ConversationBase {
    /// Create a new conversation base
    pub fn new(conversation_id: String, role: Role) -> Self {
        Self {
            conversation_id,
            role,
            participants: Vec::new(),
            messages: Vec::new(),
            start_time: chrono::Utc::now().timestamp_millis(),
            deadline: None,
        }
    }

    /// Add a participant
    pub fn add_participant(&mut self, agent: proto::AgentId) {
        if !self.participants.iter().any(|p| p.name == agent.name) {
            self.participants.push(agent);
        }
    }

    /// Record a message
    pub fn record_message(&mut self, msg: proto::AclMessage) {
        self.messages.push(msg);
    }

    /// Check if deadline has passed
    pub fn is_expired(&self) -> bool {
        if let Some(deadline) = self.deadline {
            chrono::Utc::now().timestamp_millis() > deadline
        } else {
            false
        }
    }
}

/// Helper to create a response message
pub fn create_response(
    original: &proto::AclMessage,
    performative: proto::Performative,
    content: Vec<u8>,
) -> proto::AclMessage {
    proto::AclMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        performative: performative as i32,
        sender: original.receivers.first().cloned(),
        receivers: original.sender.clone().map(|s| vec![s]).unwrap_or_default(),
        reply_to: None,
        protocol: original.protocol,
        conversation_id: original.conversation_id.clone(),
        in_reply_to: Some(original.message_id.clone()),
        reply_with: None,
        reply_by: None,
        language: original.language.clone(),
        encoding: original.encoding.clone(),
        ontology: original.ontology.clone(),
        content,
        user_properties: std::collections::HashMap::new(),
    }
}

/// Create a protocol state machine from protocol type
pub fn create_state_machine(
    protocol: proto::ProtocolType,
) -> Result<Box<dyn ProtocolStateMachine>, ProtocolError> {
    match protocol {
        proto::ProtocolType::ProtocolRequest => {
            Ok(Box::new(super::request::RequestProtocol::new(Role::Participant)))
        }
        proto::ProtocolType::ProtocolQuery => {
            Ok(Box::new(super::query::QueryProtocol::new(Role::Participant)))
        }
        proto::ProtocolType::ProtocolContractNet => {
            Ok(Box::new(super::contract_net::ContractNetProtocol::new(Role::Participant)))
        }
        proto::ProtocolType::ProtocolSubscribe => {
            Ok(Box::new(super::subscribe::SubscribeProtocol::new(Role::Participant)))
        }
        _ => Err(ProtocolError::NotSupported(protocol)),
    }
}
