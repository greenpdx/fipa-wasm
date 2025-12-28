// lib.rs - FIPA WASM Distributed Agent System
//
// A comprehensive implementation of FIPA protocols with mobile WASM agents
// that can migrate between distributed nodes.

#![doc = include_str!("../README.md")]

pub mod actor;
pub mod behavior;
pub mod consensus;
pub mod content;
pub mod interplatform;
pub mod network;
pub mod observability;
pub mod persistence;
pub mod platform;
pub mod proto;
pub mod protocol;
pub mod security;
pub mod tools;
pub mod trust;
pub mod wasm;

// Re-export commonly used types
pub use actor::{
    AgentActor, AgentConfig, AgentError, AgentInfo, AgentRuntimeState, AgentSnapshot,
    AgentStatus, ActorRegistry, DeliverMessage, MigrationReason, RestartStrategy,
    ShutdownReason, SpawnAgent, Supervisor,
};

pub use protocol::{
    CompletionData, ContractNetProtocol, ProcessResult, ProtocolError, ProtocolStateMachine,
    QueryProtocol, RequestProtocol, Role, SubscribeProtocol,
};

pub use wasm::WasmRuntime;

pub use observability::{
    init_metrics, init_tracing, record_agent_spawned, record_agent_stopped,
    record_consensus_commit, record_consensus_election, record_message_latency,
    record_message_received, record_message_sent, record_migration, record_wasm_execution,
    MetricsConfig, MetricsHandle, TracingConfig, TracingFormat,
};

pub use security::{
    AuthError, AuthResult, Authenticator, Session, SessionId,
    AgentCredentials, Certificate, Token, TokenType,
    Action as SecurityAction, Permission, PermissionCheck, PermissionError, PermissionSet,
    Resource as SecurityResource,
    Policy, PolicyEngine, PolicyError, Role as SecurityRole, RoleBinding, SecurityPolicy,
    SecurityConfig, SecurityManager,
};

pub use content::{
    Codec, CodecError, CodecRegistry, ContentElement, ContentError, ContentManager,
    Concept, Predicate, Action as ContentAction, Term, Ontology, OntologyError,
    OntologyRegistry, Schema, SchemaField, SchemaType, SlCodec,
};

pub use persistence::{
    PersistenceConfig, PersistenceError, PersistenceManager, PersistenceStats,
    AgentSnapshot as PersistedAgentSnapshot, ConversationSnapshot, PlatformSnapshot,
    ServiceSnapshot, SnapshotId, SnapshotMetadata,
    RecoveryEngine, RecoveryError, RecoveryState, RecoveredAgent,
    FileStorage, MemoryStorage, Storage, StorageError,
};

pub use interplatform::{
    Acc, AccConfig, AccError, AccStats,
    AgentAddress, PlatformAddress, AddressResolver, AddressError,
    MessageEnvelope, EnvelopeBuilder, TransportInfo,
    Mtp, MtpConfig, MtpError, MtpRegistry, MtpStatus, HttpMtp,
    InterplatformConfig, InterplatformError,
};

// Legacy compatibility - re-export old types with deprecation warnings
#[deprecated(since = "0.2.0", note = "Use proto::AgentId instead")]
pub type AgentId = proto::AgentId;

#[deprecated(since = "0.2.0", note = "Use proto::Performative instead")]
pub type Performative = proto::Performative;

#[deprecated(since = "0.2.0", note = "Use proto::ProtocolType instead")]
pub type ProtocolType = proto::ProtocolType;

#[deprecated(since = "0.2.0", note = "Use proto::AclMessage instead")]
pub type AclMessage = proto::AclMessage;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::actor::{
        AgentActor, AgentConfig, AgentError, AgentSnapshot, AgentStatus, ActorRegistry,
        DeliverMessage, RestartStrategy, ShutdownReason, SpawnAgent, Supervisor,
    };
    pub use crate::proto::{
        AclMessage, AgentCapabilities, AgentId, Performative, ProtocolType, ServiceDescription,
    };
    pub use crate::protocol::{
        CompletionData, ContractNetProtocol, ProcessResult, ProtocolError, ProtocolStateMachine,
        QueryProtocol, RequestProtocol, Role, SubscribeProtocol,
    };
    pub use crate::wasm::WasmRuntime;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!VERSION.is_empty());
    }
}
