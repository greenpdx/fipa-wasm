// persistence/snapshot.rs - Snapshot Types and Serialization
//
//! Snapshot data structures for persistence
//!
//! Provides:
//! - Agent state snapshots
//! - Conversation snapshots
//! - Platform-wide snapshots
//! - Service registry snapshots

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use thiserror::Error;

/// Snapshot errors
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("Snapshot corrupted: {0}")]
    Corrupted(String),

    #[error("Snapshot not found: {0}")]
    NotFound(String),

    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u32, actual: u32 },
}

/// Current snapshot format version
pub const SNAPSHOT_VERSION: u32 = 1;

/// Unique snapshot identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotId {
    /// Sequential ID
    pub id: u64,
    /// Timestamp when created
    pub timestamp: u64,
    /// Unique suffix for disambiguation
    pub suffix: String,
}

impl SnapshotId {
    /// Create a new snapshot ID
    pub fn new(sequence: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id: sequence,
            timestamp,
            suffix: format!("{:08x}", rand::random::<u32>()),
        }
    }

    /// Create a disabled/placeholder ID
    pub fn disabled() -> Self {
        Self {
            id: 0,
            timestamp: 0,
            suffix: "disabled".to_string(),
        }
    }

    /// Check if this is a disabled ID
    pub fn is_disabled(&self) -> bool {
        self.suffix == "disabled"
    }

    /// Convert to a filename-safe string
    pub fn to_filename(&self) -> String {
        format!("{}_{}", self.timestamp, self.suffix)
    }

    /// Parse from a filename
    pub fn from_filename(filename: &str) -> Option<Self> {
        let parts: Vec<&str> = filename.split('_').collect();
        if parts.len() >= 2 {
            let timestamp = parts[0].parse().ok()?;
            let suffix = parts[1].to_string();
            Some(Self {
                id: 0, // Will be updated when loading
                timestamp,
                suffix,
            })
        } else {
            None
        }
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "snap-{}-{}", self.id, self.suffix)
    }
}

/// Snapshot metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Snapshot format version
    pub version: u32,

    /// Creation timestamp
    pub created_at: u64,

    /// Agent name (if applicable)
    pub agent_name: Option<String>,

    /// Snapshot ID
    pub snapshot_id: SnapshotId,

    /// Size in bytes (of serialized data)
    pub size_bytes: u64,

    /// Whether the snapshot is compressed
    pub compressed: bool,

    /// Checksum for integrity verification
    pub checksum: Option<String>,

    /// Additional metadata
    pub extra: HashMap<String, String>,
}

impl SnapshotMetadata {
    /// Create metadata for the current time
    pub fn now() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            version: SNAPSHOT_VERSION,
            created_at: timestamp,
            agent_name: None,
            snapshot_id: SnapshotId::disabled(),
            size_bytes: 0,
            compressed: false,
            checksum: None,
            extra: HashMap::new(),
        }
    }

    /// Set the agent name
    pub fn with_agent(mut self, name: &str) -> Self {
        self.agent_name = Some(name.to_string());
        self
    }

    /// Set the snapshot ID
    pub fn with_id(mut self, id: SnapshotId) -> Self {
        self.snapshot_id = id;
        self
    }

    /// Add extra metadata
    pub fn with_extra(mut self, key: &str, value: &str) -> Self {
        self.extra.insert(key.to_string(), value.to_string());
        self
    }

    /// Calculate and set checksum
    pub fn with_checksum(mut self, data: &[u8]) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        self.checksum = Some(format!("{:016x}", hasher.finish()));
        self
    }

    /// Verify checksum
    pub fn verify_checksum(&self, data: &[u8]) -> bool {
        if let Some(ref expected) = self.checksum {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            data.hash(&mut hasher);
            let actual = format!("{:016x}", hasher.finish());
            &actual == expected
        } else {
            true // No checksum to verify
        }
    }
}

/// Agent state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// Snapshot ID
    pub id: SnapshotId,

    /// Agent name
    pub agent_name: String,

    /// Metadata
    pub metadata: SnapshotMetadata,

    /// WASM instance state (serialized)
    pub wasm_state: Vec<u8>,

    /// Active conversations
    pub conversations: Vec<ConversationSnapshot>,

    /// Pending messages (not yet processed)
    pub pending_messages: Vec<MessageSnapshot>,
}

impl AgentSnapshot {
    /// Create a new agent snapshot
    pub fn new(agent_name: &str, wasm_state: Vec<u8>) -> Self {
        let id = SnapshotId::new(0);
        Self {
            id: id.clone(),
            agent_name: agent_name.to_string(),
            metadata: SnapshotMetadata::now()
                .with_agent(agent_name)
                .with_id(id),
            wasm_state,
            conversations: vec![],
            pending_messages: vec![],
        }
    }

    /// Add a conversation
    pub fn with_conversation(mut self, conversation: ConversationSnapshot) -> Self {
        self.conversations.push(conversation);
        self
    }

    /// Add a pending message
    pub fn with_pending_message(mut self, message: MessageSnapshot) -> Self {
        self.pending_messages.push(message);
        self
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        serde_json::to_vec(self).map_err(|e| SnapshotError::SerializationFailed(e.to_string()))
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        serde_json::from_slice(bytes).map_err(|e| SnapshotError::DeserializationFailed(e.to_string()))
    }
}

/// Conversation state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSnapshot {
    /// Conversation ID
    pub conversation_id: String,

    /// Protocol type
    pub protocol: String,

    /// Current state name
    pub state: String,

    /// Role in the conversation
    pub role: String,

    /// Other participants
    pub participants: Vec<String>,

    /// Message history (IDs or full messages)
    pub message_ids: Vec<String>,

    /// Protocol-specific state data
    pub state_data: Vec<u8>,

    /// Started at timestamp
    pub started_at: u64,

    /// Last activity timestamp
    pub last_activity: u64,
}

impl ConversationSnapshot {
    /// Create a new conversation snapshot
    pub fn new(conversation_id: &str, protocol: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            conversation_id: conversation_id.to_string(),
            protocol: protocol.to_string(),
            state: "unknown".to_string(),
            role: "unknown".to_string(),
            participants: vec![],
            message_ids: vec![],
            state_data: vec![],
            started_at: now,
            last_activity: now,
        }
    }

    /// Set the current state
    pub fn with_state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Set the role
    pub fn with_role(mut self, role: &str) -> Self {
        self.role = role.to_string();
        self
    }

    /// Add a participant
    pub fn with_participant(mut self, participant: &str) -> Self {
        self.participants.push(participant.to_string());
        self
    }
}

/// Message snapshot (for pending messages)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSnapshot {
    /// Message ID
    pub message_id: String,

    /// Sender agent
    pub sender: String,

    /// Receiver agent
    pub receiver: String,

    /// Performative
    pub performative: String,

    /// Content (serialized)
    pub content: Vec<u8>,

    /// Conversation ID
    pub conversation_id: Option<String>,

    /// Timestamp
    pub timestamp: u64,
}

impl MessageSnapshot {
    /// Create a new message snapshot
    pub fn new(message_id: &str, sender: &str, receiver: &str) -> Self {
        Self {
            message_id: message_id.to_string(),
            sender: sender.to_string(),
            receiver: receiver.to_string(),
            performative: "unknown".to_string(),
            content: vec![],
            conversation_id: None,
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

/// Service registration snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    /// Service name
    pub name: String,

    /// Service type
    pub service_type: Option<String>,

    /// Provider agent
    pub provider: String,

    /// Supported protocols
    pub protocols: Vec<String>,

    /// Supported ontologies
    pub ontologies: Vec<String>,

    /// Service properties
    pub properties: HashMap<String, String>,

    /// Registration timestamp
    pub registered_at: u64,
}

impl ServiceSnapshot {
    /// Create a new service snapshot
    pub fn new(name: &str, provider: &str) -> Self {
        Self {
            name: name.to_string(),
            service_type: None,
            provider: provider.to_string(),
            protocols: vec![],
            ontologies: vec![],
            properties: HashMap::new(),
            registered_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Set service type
    pub fn with_type(mut self, service_type: &str) -> Self {
        self.service_type = Some(service_type.to_string());
        self
    }

    /// Add a protocol
    pub fn with_protocol(mut self, protocol: &str) -> Self {
        self.protocols.push(protocol.to_string());
        self
    }
}

/// Platform-wide snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSnapshot {
    /// Snapshot version
    pub version: u32,

    /// Platform name
    pub platform_name: String,

    /// Creation timestamp
    pub created_at: u64,

    /// Node ID (if in cluster)
    pub node_id: Option<String>,

    /// Cluster configuration
    pub cluster_config: Option<ClusterConfig>,

    /// Agent names registered on this platform
    pub agent_names: Vec<String>,

    /// Platform configuration
    pub config: HashMap<String, String>,
}

impl PlatformSnapshot {
    /// Create a new platform snapshot
    pub fn new(platform_name: &str) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            platform_name: platform_name.to_string(),
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            node_id: None,
            cluster_config: None,
            agent_names: vec![],
            config: HashMap::new(),
        }
    }

    /// Set node ID
    pub fn with_node_id(mut self, node_id: &str) -> Self {
        self.node_id = Some(node_id.to_string());
        self
    }

    /// Add an agent name
    pub fn with_agent(mut self, agent_name: &str) -> Self {
        self.agent_names.push(agent_name.to_string());
        self
    }

    /// Set a configuration value
    pub fn with_config(mut self, key: &str, value: &str) -> Self {
        self.config.insert(key.to_string(), value.to_string());
        self
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        serde_json::to_vec(self).map_err(|e| SnapshotError::SerializationFailed(e.to_string()))
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        serde_json::from_slice(bytes).map_err(|e| SnapshotError::DeserializationFailed(e.to_string()))
    }
}

/// Cluster configuration snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Cluster name
    pub cluster_name: String,

    /// Known peer addresses
    pub peers: Vec<String>,

    /// Leader node ID (if known)
    pub leader_id: Option<String>,

    /// Current term (Raft)
    pub term: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_id_creation() {
        let id = SnapshotId::new(1);
        assert_eq!(id.id, 1);
        assert!(!id.is_disabled());
    }

    #[test]
    fn test_snapshot_id_disabled() {
        let id = SnapshotId::disabled();
        assert!(id.is_disabled());
    }

    #[test]
    fn test_snapshot_id_filename() {
        let id = SnapshotId::new(1);
        let filename = id.to_filename();
        assert!(filename.contains('_'));

        let parsed = SnapshotId::from_filename(&filename);
        assert!(parsed.is_some());
    }

    #[test]
    fn test_agent_snapshot_serialization() {
        let snapshot = AgentSnapshot::new("test-agent", vec![1, 2, 3, 4])
            .with_conversation(
                ConversationSnapshot::new("conv-1", "request")
                    .with_state("awaiting-response")
                    .with_role("initiator"),
            );

        let bytes = snapshot.to_bytes().unwrap();
        let restored = AgentSnapshot::from_bytes(&bytes).unwrap();

        assert_eq!(restored.agent_name, "test-agent");
        assert_eq!(restored.wasm_state, vec![1, 2, 3, 4]);
        assert_eq!(restored.conversations.len(), 1);
    }

    #[test]
    fn test_metadata_checksum() {
        let data = b"test data";
        let metadata = SnapshotMetadata::now().with_checksum(data);

        assert!(metadata.checksum.is_some());
        assert!(metadata.verify_checksum(data));
        assert!(!metadata.verify_checksum(b"different data"));
    }

    #[test]
    fn test_platform_snapshot() {
        let snapshot = PlatformSnapshot::new("test-platform")
            .with_node_id("node-1")
            .with_agent("agent-1")
            .with_agent("agent-2")
            .with_config("key", "value");

        assert_eq!(snapshot.platform_name, "test-platform");
        assert_eq!(snapshot.agent_names.len(), 2);
        assert_eq!(snapshot.config.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_service_snapshot() {
        let service = ServiceSnapshot::new("calculator", "calc-agent")
            .with_type("computation")
            .with_protocol("fipa-request");

        assert_eq!(service.name, "calculator");
        assert_eq!(service.provider, "calc-agent");
        assert_eq!(service.service_type, Some("computation".to_string()));
    }
}
