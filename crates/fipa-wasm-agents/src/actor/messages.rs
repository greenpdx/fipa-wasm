// actor/messages.rs - Inter-actor message types

use actix::prelude::*;
use crate::proto;
use crate::protocol::ProtocolError;
use std::time::Duration;

/// Deliver an ACL message to an agent
#[derive(Message, Clone, Debug)]
#[rtype(result = "Result<(), AgentError>")]
pub struct DeliverMessage {
    pub message: proto::AclMessage,
}

/// Request agent state capture for migration
#[derive(Message)]
#[rtype(result = "Result<AgentSnapshot, AgentError>")]
pub struct CaptureState;

/// Initiate migration to another node
#[derive(Message)]
#[rtype(result = "Result<(), AgentError>")]
pub struct MigrateTo {
    pub target_node: String,
    pub reason: MigrationReason,
}

/// Request graceful shutdown
#[derive(Message)]
#[rtype(result = "()")]
pub struct Shutdown {
    pub reason: ShutdownReason,
}

/// Query agent status
#[derive(Message)]
#[rtype(result = "AgentStatus")]
pub struct GetStatus;

/// Register a service this agent provides
#[derive(Message)]
#[rtype(result = "Result<(), AgentError>")]
pub struct RegisterService {
    pub service: proto::ServiceDescription,
}

/// Find agents by service name
#[derive(Message)]
#[rtype(result = "Result<Vec<proto::AgentId>, AgentError>")]
pub struct FindAgents {
    pub service_name: String,
    pub protocol: Option<proto::ProtocolType>,
}

/// Start a new conversation
#[derive(Message)]
#[rtype(result = "Result<String, AgentError>")]
pub struct StartConversation {
    pub protocol: proto::ProtocolType,
    pub participants: Vec<proto::AgentId>,
}

// =============================================================================
// Supervision Messages
// =============================================================================

/// Notification of supervision events
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct SupervisionEvent {
    pub agent_id: proto::AgentId,
    pub event: SupervisionEventType,
}

/// Supervision event types
#[derive(Clone, Debug)]
pub enum SupervisionEventType {
    Started,
    Stopped,
    Failed { error: String, will_restart: bool },
    Migrated { from_node: String, to_node: String },
    Recovered,
}

/// Request to spawn a new agent
#[derive(Message)]
#[rtype(result = "Result<Addr<super::AgentActor>, AgentError>")]
pub struct SpawnAgent {
    pub config: AgentConfig,
}

/// Request to stop an agent
#[derive(Message)]
#[rtype(result = "Result<(), AgentError>")]
pub struct StopAgent {
    pub agent_id: proto::AgentId,
    pub reason: ShutdownReason,
}

/// Get list of all supervised agents
#[derive(Message)]
#[rtype(result = "Vec<AgentInfo>")]
pub struct ListAgents;

// =============================================================================
// Registry Messages
// =============================================================================

/// Register an actor address
#[derive(Message)]
#[rtype(result = "()")]
pub struct RegisterActor {
    pub agent_id: proto::AgentId,
    pub addr: Addr<super::AgentActor>,
}

/// Deregister an actor
#[derive(Message)]
#[rtype(result = "()")]
pub struct DeregisterActor {
    pub agent_id: proto::AgentId,
}

/// Lookup an actor by agent ID
#[derive(Message)]
#[rtype(result = "Option<Addr<super::AgentActor>>")]
pub struct LookupActor {
    pub agent_id: proto::AgentId,
}

/// Resolve agent to its current node
#[derive(Message)]
#[rtype(result = "Result<String, AgentError>")]
pub struct ResolveAgent {
    pub agent_id: proto::AgentId,
}

// =============================================================================
// Network Messages
// =============================================================================

/// Send a message to a remote agent
#[derive(Message)]
#[rtype(result = "Result<String, AgentError>")]
pub struct SendRemoteMessage {
    pub target_node: String,
    pub message: proto::AclMessage,
}

/// Incoming message from network
#[derive(Message)]
#[rtype(result = "()")]
pub struct IncomingMessage {
    pub envelope: proto::MessageEnvelope,
}

/// Node discovered on network
#[derive(Message)]
#[rtype(result = "()")]
pub struct NodeDiscovered {
    pub node_id: String,
    pub addresses: Vec<String>,
}

/// Node disconnected
#[derive(Message)]
#[rtype(result = "()")]
pub struct NodeDisconnected {
    pub node_id: String,
}

// =============================================================================
// Supporting Types
// =============================================================================

/// Agent error types
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error("Agent not found: {0}")]
    NotFound(String),

    #[error("Protocol not allowed: {0:?}")]
    ProtocolNotAllowed(proto::ProtocolType),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Migration failed: {0}")]
    MigrationFailed(String),

    #[error("WASM runtime error: {0}")]
    RuntimeError(String),

    #[error("Capability denied: {0}")]
    CapabilityDenied(String),

    #[error("Timeout")]
    Timeout,

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Actor mailbox error")]
    MailboxError,

    #[error("Protocol error: {0}")]
    ProtocolError(String),
}

impl From<ProtocolError> for AgentError {
    fn from(err: ProtocolError) -> Self {
        AgentError::ProtocolError(err.to_string())
    }
}

/// Migration reasons
#[derive(Clone, Debug)]
pub enum MigrationReason {
    UserRequested,
    LoadBalancing,
    NetworkOptimization,
    FollowData { data_location: String },
    NodeShutdown,
}

impl From<MigrationReason> for proto::MigrationReason {
    fn from(reason: MigrationReason) -> Self {
        match reason {
            MigrationReason::UserRequested => proto::MigrationReason::UserRequested,
            MigrationReason::LoadBalancing => proto::MigrationReason::LoadBalancing,
            MigrationReason::NetworkOptimization => proto::MigrationReason::NetworkOptimization,
            MigrationReason::FollowData { .. } => proto::MigrationReason::FollowData,
            MigrationReason::NodeShutdown => proto::MigrationReason::NodeShutdown,
        }
    }
}

/// Shutdown reasons
#[derive(Clone, Debug)]
pub enum ShutdownReason {
    Requested,
    Migration,
    NodeShutdown,
    Error(String),
    Timeout,
}

/// Agent configuration for spawning
#[derive(Clone)]
pub struct AgentConfig {
    pub id: proto::AgentId,
    pub wasm_module: Vec<u8>,
    pub capabilities: proto::AgentCapabilities,
    pub initial_state: Option<proto::AgentState>,
    pub restart_strategy: RestartStrategy,
}

/// Restart strategies for supervision
#[derive(Clone, Debug)]
pub enum RestartStrategy {
    /// Always restart immediately
    Immediate,

    /// Restart with exponential backoff
    Backoff {
        initial: Duration,
        max: Duration,
        multiplier: f64,
    },

    /// Stop after N failures in time window
    MaxFailures {
        count: u32,
        window: Duration,
    },

    /// Never restart
    Never,
}

impl Default for RestartStrategy {
    fn default() -> Self {
        RestartStrategy::Backoff {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            multiplier: 2.0,
        }
    }
}

/// Agent state snapshot for migration
#[derive(Clone)]
pub struct AgentSnapshot {
    pub agent_id: proto::AgentId,
    pub wasm_module: Vec<u8>,
    pub wasm_hash: [u8; 32],
    pub state: proto::AgentState,
    pub capabilities: proto::AgentCapabilities,
    pub migration_history: Vec<String>,
}

impl AgentSnapshot {
    /// Compute SHA-256 hash of the snapshot for signing
    pub fn compute_hash(&self) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&self.wasm_module);
        hasher.update(&self.state.memory);
        hasher.finalize().into()
    }
}

/// Agent runtime status
#[derive(Clone, Debug)]
pub struct AgentStatus {
    pub agent_id: proto::AgentId,
    pub state: AgentRuntimeState,
    pub active_conversations: usize,
    pub messages_processed: u64,
    pub uptime_secs: u64,
    pub memory_used: usize,
}

impl<A, M> actix::dev::MessageResponse<A, M> for AgentStatus
where
    A: actix::Actor,
    M: actix::Message<Result = AgentStatus>,
{
    fn handle(self, _ctx: &mut A::Context, tx: Option<actix::dev::OneshotSender<M::Result>>) {
        if let Some(tx) = tx {
            let _ = tx.send(self);
        }
    }
}

/// Agent runtime states
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRuntimeState {
    Starting,
    Running,
    Paused,
    Migrating,
    Stopping,
    Stopped,
    Failed,
}

/// Agent info for listing
#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub agent_id: proto::AgentId,
    pub state: AgentRuntimeState,
    pub restart_count: u32,
    pub last_error: Option<String>,
}
