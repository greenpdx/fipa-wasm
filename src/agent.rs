// agent.rs
// Mobile agent structures and WASM runtime

use crate::acl_message::*;
use crate::protocols::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Node identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

/// Mobile agent definition
#[derive(Clone, Serialize, Deserialize)]
pub struct MobileAgent {
    pub id: AgentId,
    pub wasm_module: Vec<u8>,
    pub state: AgentState,
    pub capabilities: Capabilities,
    pub migration_history: Vec<NodeId>,
    pub signature: Option<Vec<u8>>,
}

/// Agent state that migrates with the agent
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub memory_snapshot: Vec<u8>,
    pub globals: HashMap<String, GlobalValue>,
    pub conversations: HashMap<ConversationId, ConversationSnapshot>,
    pub custom_data: Vec<u8>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            memory_snapshot: Vec::new(),
            globals: HashMap::new(),
            conversations: HashMap::new(),
            custom_data: Vec::new(),
        }
    }
}

/// Global variable value
#[derive(Clone, Serialize, Deserialize)]
pub enum GlobalValue {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

/// Conversation state snapshot
#[derive(Clone, Serialize, Deserialize)]
pub struct ConversationSnapshot {
    pub protocol: ProtocolType,
    pub state: String, // Serialized state
    pub messages: Vec<AclMessage>,
}

/// Agent capabilities and permissions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub max_memory_bytes: usize,
    pub max_execution_time_ms: u64,
    pub allowed_protocols: HashSet<ProtocolType>,
    pub network_access: NetworkAccess,
    pub storage_quota_bytes: usize,
    pub migration_allowed: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_execution_time_ms: 5000,        // 5 seconds
            allowed_protocols: HashSet::new(),
            network_access: NetworkAccess::LocalOnly,
            storage_quota_bytes: 10 * 1024 * 1024, // 10 MB
            migration_allowed: false,
        }
    }
}

/// Network access levels
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkAccess {
    None,
    LocalOnly,
    Restricted(Vec<String>),
    Unrestricted,
}

/// Agent package for migration
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentPackage {
    pub agent: MobileAgent,
    pub verification: PackageVerification,
}

/// Package verification data
#[derive(Clone, Serialize, Deserialize)]
pub struct PackageVerification {
    pub hash: [u8; 32],
    pub signature: Vec<u8>,
    pub signer_public_key: Vec<u8>,
    pub timestamp: Timestamp,
}

impl AgentPackage {
    pub fn new(agent: MobileAgent) -> Self {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(&agent.wasm_module);
        hasher.update(&bincode::serialize(&agent.state).unwrap_or_default());
        let hash: [u8; 32] = hasher.finalize().into();

        Self {
            agent,
            verification: PackageVerification {
                hash,
                signature: Vec::new(), // Would be signed in production
                signer_public_key: Vec::new(),
                timestamp: Timestamp::now(),
            },
        }
    }

    pub fn verify(&self) -> Result<bool, String> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(&self.agent.wasm_module);
        hasher.update(&bincode::serialize(&self.agent.state).unwrap_or_default());
        let computed_hash: [u8; 32] = hasher.finalize().into();

        if computed_hash != self.verification.hash {
            return Ok(false);
        }

        // In production, verify Ed25519 signature here
        Ok(true)
    }
}

/// Migration reasons
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MigrationReason {
    UserRequested,
    LoadBalancing,
    NetworkOptimization,
    FollowData,
    Shutdown,
}

/// Migration metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationMetadata {
    pub source_node: NodeId,
    pub target_node: NodeId,
    pub agent_id: AgentId,
    pub reason: MigrationReason,
    pub timestamp: Timestamp,
}

/// Agent directory entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub id: AgentId,
    pub current_node: NodeId,
    pub capabilities: Capabilities,
    pub services: Vec<ServiceDescription>,
    pub load: LoadMetrics,
}

/// Service description for agent capabilities
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceDescription {
    pub name: String,
    pub protocols: Vec<ProtocolType>,
    pub ontology: String,
    pub description: String,
}

/// Agent load metrics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadMetrics {
    pub active_conversations: usize,
    pub cpu_usage_percent: f32,
    pub memory_usage_bytes: usize,
}

impl Default for LoadMetrics {
    fn default() -> Self {
        Self {
            active_conversations: 0,
            cpu_usage_percent: 0.0,
            memory_usage_bytes: 0,
        }
    }
}

/// Conversation role
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Initiator,
    Participant,
    Broker,
}

/// Generic conversation handler
pub struct GenericConversation<P: Protocol> {
    pub protocol: P,
    pub state: P::State,
    pub metadata: P::Metadata,
    pub role: Role,
    pub messages: Vec<AclMessage>,
}

impl<P: Protocol> GenericConversation<P> {
    pub fn new(
        protocol: P,
        initial_state: P::State,
        metadata: P::Metadata,
        role: Role,
    ) -> Self {
        Self {
            protocol,
            state: initial_state,
            metadata,
            role,
            messages: Vec::new(),
        }
    }

    pub fn process_message(
        &mut self,
        msg: AclMessage,
    ) -> Result<Option<AclMessage>, ProtocolError> {
        // Validate message
        self.protocol.validate_message(&msg, &self.state)?;

        // Transition state
        let new_state = self.protocol.transition(self.state.clone(), &msg)?;
        self.state = new_state;

        // Store message
        self.messages.push(msg);

        // Generate response based on role and state
        Ok(None)
    }

    pub fn is_complete(&self) -> bool {
        self.protocol.is_terminal(&self.state)
    }
}

/// Conversation handler trait
pub trait ConversationHandler: Send + Sync {
    fn handle_message(
        &mut self,
        msg: AclMessage,
    ) -> Result<Option<AclMessage>, ProtocolError>;
    fn get_state(&self) -> String;
    fn is_complete(&self) -> bool;
}

/// Conversation manager
pub struct ConversationManager {
    pub conversations: HashMap<ConversationId, Box<dyn ConversationHandler>>,
}

impl ConversationManager {
    pub fn new() -> Self {
        Self {
            conversations: HashMap::new(),
        }
    }

    pub fn add_conversation(
        &mut self,
        conv_id: ConversationId,
        handler: Box<dyn ConversationHandler>,
    ) {
        self.conversations.insert(conv_id, handler);
    }

    pub fn handle_message(
        &mut self,
        msg: AclMessage,
    ) -> Result<Option<AclMessage>, ProtocolError> {
        let conv_id = msg
            .header
            .conversation_id
            .as_ref()
            .ok_or(ProtocolError::MissingConversationId)?;

        if let Some(handler) = self.conversations.get_mut(conv_id) {
            handler.handle_message(msg)
        } else {
            Err(ProtocolError::UnknownConversation)
        }
    }

    pub fn cleanup_completed(&mut self) {
        self.conversations.retain(|_, handler| !handler.is_complete());
    }
}

impl Default for ConversationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_package_creation() {
        let agent = MobileAgent {
            id: AgentId::new("test-agent"),
            wasm_module: vec![0x00, 0x61, 0x73, 0x6D], // WASM magic number
            state: AgentState::default(),
            capabilities: Capabilities::default(),
            migration_history: vec![],
            signature: None,
        };

        let package = AgentPackage::new(agent);
        assert!(package.verify().unwrap());
    }

    #[test]
    fn test_conversation_manager() {
        let mut manager = ConversationManager::new();
        assert_eq!(manager.conversations.len(), 0);

        manager.cleanup_completed();
        assert_eq!(manager.conversations.len(), 0);
    }
}
