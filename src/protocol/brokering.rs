// protocol/brokering.rs - FIPA Brokering Protocol
//
//! FIPA Brokering Protocol Implementation
//!
//! The Brokering protocol allows a broker agent to mediate interactions
//! between agents. The broker:
//! - Receives requests from initiators
//! - Finds suitable service providers
//! - Forwards requests and consolidates responses
//!
//! # Protocol Flow
//!
//! ```text
//! Initiator         Broker             Provider(s)
//!     |                |                    |
//!     |--- PROXY ----->|                    |
//!     |                |                    |
//!     |                |--- REQUEST ------->|
//!     |                |                    |
//!     |                |<-- AGREE/REFUSE ---|
//!     |                |                    |
//!     |                |<-- INFORM ---------|
//!     |                |                    |
//!     |<-- INFORM -----|                    |
//!     |                |                    |
//! ```

use super::state_machine::*;
use crate::proto;
use std::collections::HashMap;

/// Broker request status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    /// Request forwarded, awaiting response
    Pending,
    /// Provider agreed to handle
    Agreed,
    /// Provider refused
    Refused,
    /// Provider completed successfully
    Completed,
    /// Provider failed
    Failed,
}

/// Provider tracking
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    /// Provider agent ID
    pub agent_id: proto::AgentId,
    /// Status
    pub status: ProviderStatus,
    /// Response content (if any)
    pub response: Option<Vec<u8>>,
}

/// Brokering Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokeringState {
    /// Initial state
    NotStarted,
    /// Proxy request received
    ProxyReceived,
    /// Forwarding to providers
    Forwarding,
    /// Waiting for provider responses
    WaitingResponses,
    /// Consolidating responses
    Consolidating,
    /// Completed successfully
    Completed,
    /// Failed (no providers or all failed)
    Failed,
    /// Cancelled
    Cancelled,
}

impl BrokeringState {
    pub fn as_str(&self) -> &'static str {
        match self {
            BrokeringState::NotStarted => "not_started",
            BrokeringState::ProxyReceived => "proxy_received",
            BrokeringState::Forwarding => "forwarding",
            BrokeringState::WaitingResponses => "waiting_responses",
            BrokeringState::Consolidating => "consolidating",
            BrokeringState::Completed => "completed",
            BrokeringState::Failed => "failed",
            BrokeringState::Cancelled => "cancelled",
        }
    }
}

/// Brokering Protocol Implementation
#[derive(Debug)]
pub struct BrokeringProtocol {
    /// Current state
    state: BrokeringState,

    /// Conversation base
    base: ConversationBase,

    /// Original request from initiator
    original_request: Option<proto::AclMessage>,

    /// Initiator agent ID
    initiator: Option<proto::AgentId>,

    /// Provider tracking
    providers: HashMap<String, ProviderInfo>,

    /// Required number of responses (None = all)
    required_responses: Option<usize>,

    /// Consolidated results
    results: Vec<Vec<u8>>,

    /// Service name being brokered
    service_name: Option<String>,
}

impl BrokeringProtocol {
    /// Create a new brokering protocol (as broker)
    pub fn new_as_broker() -> Self {
        Self {
            state: BrokeringState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            original_request: None,
            initiator: None,
            providers: HashMap::new(),
            required_responses: None,
            results: vec![],
            service_name: None,
        }
    }

    /// Create a new brokering protocol (as initiator)
    pub fn new_as_initiator() -> Self {
        Self {
            state: BrokeringState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            original_request: None,
            initiator: None,
            providers: HashMap::new(),
            required_responses: None,
            results: vec![],
            service_name: None,
        }
    }

    /// Set required number of responses
    pub fn with_required_responses(mut self, count: usize) -> Self {
        self.required_responses = Some(count);
        self
    }

    /// Set service name
    pub fn with_service_name(mut self, name: String) -> Self {
        self.service_name = Some(name);
        self
    }

    /// Set conversation ID
    pub fn with_conversation_id(mut self, id: String) -> Self {
        self.base.conversation_id = id;
        self
    }

    /// Add a provider to forward to
    pub fn add_provider(&mut self, agent_id: proto::AgentId) {
        self.providers.insert(agent_id.name.clone(), ProviderInfo {
            agent_id,
            status: ProviderStatus::Pending,
            response: None,
        });
    }

    /// Update provider status
    pub fn update_provider(&mut self, name: &str, status: ProviderStatus, response: Option<Vec<u8>>) {
        if let Some(provider) = self.providers.get_mut(name) {
            provider.status = status;
            provider.response = response;
        }
    }

    /// Get providers
    pub fn providers(&self) -> &HashMap<String, ProviderInfo> {
        &self.providers
    }

    /// Get consolidated results
    pub fn results(&self) -> &[Vec<u8>] {
        &self.results
    }

    /// Check if all providers have responded
    pub fn all_responded(&self) -> bool {
        self.providers.values().all(|p| {
            matches!(
                p.status,
                ProviderStatus::Completed | ProviderStatus::Failed | ProviderStatus::Refused
            )
        })
    }

    /// Count successful responses
    pub fn successful_count(&self) -> usize {
        self.providers
            .values()
            .filter(|p| p.status == ProviderStatus::Completed)
            .count()
    }

    /// Consolidate results from providers
    pub fn consolidate(&mut self) -> Result<(), ProtocolError> {
        if !matches!(self.state, BrokeringState::WaitingResponses | BrokeringState::Consolidating) {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "consolidate".to_string(),
            });
        }

        self.results = self
            .providers
            .values()
            .filter(|p| p.status == ProviderStatus::Completed)
            .filter_map(|p| p.response.clone())
            .collect();

        if self.results.is_empty() {
            self.state = BrokeringState::Failed;
        } else {
            self.state = BrokeringState::Completed;
        }

        Ok(())
    }

    /// Validate state transition
    fn validate_transition(&self, performative: proto::Performative) -> Result<BrokeringState, ProtocolError> {
        use proto::Performative::*;
        use BrokeringState::*;

        match (&self.state, performative) {
            // Initiator sends proxy request
            (NotStarted, Proxy) => Ok(ProxyReceived),
            // Broker forwards to providers
            (ProxyReceived, Request) => Ok(Forwarding),
            (Forwarding, Request) => Ok(Forwarding), // Additional forwards
            // Provider responses
            (Forwarding, Agree) | (WaitingResponses, Agree) => Ok(WaitingResponses),
            (Forwarding, Refuse) | (WaitingResponses, Refuse) => Ok(WaitingResponses),
            (WaitingResponses, InformResult) | (WaitingResponses, InformDone) => Ok(Consolidating),
            (WaitingResponses, Failure) => Ok(WaitingResponses),
            // Broker sends consolidated response
            (Consolidating, Inform) => Ok(Completed),
            // Failure cases
            (_, Failure) => Ok(Failed),
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for BrokeringProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolBrokering
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
            proto::Performative::Proxy => {
                self.original_request = Some(msg.clone());
                self.initiator = msg.sender.clone();
            }
            proto::Performative::InformResult | proto::Performative::InformDone => {
                if let Some(sender) = &msg.sender {
                    self.update_provider(&sender.name, ProviderStatus::Completed, Some(msg.content.clone()));
                    self.results.push(msg.content.clone());
                }
            }
            proto::Performative::Agree => {
                if let Some(sender) = &msg.sender {
                    self.update_provider(&sender.name, ProviderStatus::Agreed, None);
                }
            }
            proto::Performative::Refuse => {
                if let Some(sender) = &msg.sender {
                    self.update_provider(&sender.name, ProviderStatus::Refused, None);
                }
            }
            proto::Performative::Failure => {
                if let Some(sender) = &msg.sender {
                    self.update_provider(&sender.name, ProviderStatus::Failed, Some(msg.content.clone()));
                }
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            BrokeringState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: Some(serde_json::to_vec(&self.results).unwrap_or_default()),
                ..Default::default()
            })),
            BrokeringState::Failed => Ok(ProcessResult::Failed("Brokering failed".into())),
            BrokeringState::Cancelled => Ok(ProcessResult::Failed("Brokering cancelled".into())),
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            BrokeringState::Completed | BrokeringState::Failed | BrokeringState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            BrokeringState::Failed | BrokeringState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            BrokeringState::NotStarted => vec![Proxy],
            BrokeringState::ProxyReceived => vec![Request, Cancel],
            BrokeringState::Forwarding => vec![Request, Agree, Refuse, Cancel],
            BrokeringState::WaitingResponses => vec![Agree, Refuse, InformResult, InformDone, Failure, Cancel],
            BrokeringState::Consolidating => vec![Inform, Cancel],
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
    fn test_brokering_basics() {
        let protocol = BrokeringProtocol::new_as_broker();
        assert_eq!(protocol.state, BrokeringState::NotStarted);
    }

    #[test]
    fn test_add_providers() {
        let mut protocol = BrokeringProtocol::new_as_broker();

        protocol.add_provider(proto::AgentId {
            name: "provider1".into(),
            addresses: vec![],
            resolvers: vec![],
        });

        protocol.add_provider(proto::AgentId {
            name: "provider2".into(),
            addresses: vec![],
            resolvers: vec![],
        });

        assert_eq!(protocol.providers().len(), 2);
    }

    #[test]
    fn test_consolidation() {
        let mut protocol = BrokeringProtocol::new_as_broker();
        protocol.state = BrokeringState::WaitingResponses;

        protocol.add_provider(proto::AgentId {
            name: "provider1".into(),
            addresses: vec![],
            resolvers: vec![],
        });

        protocol.update_provider("provider1", ProviderStatus::Completed, Some(b"result1".to_vec()));

        protocol.consolidate().unwrap();

        assert_eq!(protocol.results().len(), 1);
        assert_eq!(protocol.state, BrokeringState::Completed);
    }
}
