// lib.rs - FIPA WASM Distributed Agent System
//
// A comprehensive implementation of FIPA protocols with mobile WASM agents
// that can migrate between distributed nodes.

#![doc = include_str!("../README.md")]

pub mod actor;
pub mod consensus;
pub mod network;
pub mod observability;
pub mod proto;
pub mod protocol;
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
