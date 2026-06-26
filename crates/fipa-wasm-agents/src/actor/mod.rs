// actor/mod.rs - Actor Module

//! Actor-based agent framework using Actix.
//!
//! This module provides the actor infrastructure for running WASM agents:
//!
//! - `AgentActor` - Wraps a WASM agent instance and handles message processing
//! - `Supervisor` - Manages agent lifecycle with configurable restart strategies
//! - `ActorRegistry` - Name-based actor lookup and service discovery
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::actor::*;
//!
//! // Create supervisor
//! let supervisor = Supervisor::new("node-1".into()).start();
//!
//! // Spawn an agent
//! let agent = supervisor.send(SpawnAgent {
//!     config: AgentConfig {
//!         id: proto::AgentId { name: "my-agent".into(), .. },
//!         wasm_module: include_bytes!("agent.wasm").to_vec(),
//!         capabilities: proto::AgentCapabilities::default(),
//!         initial_state: None,
//!         restart_strategy: RestartStrategy::default(),
//!     },
//! }).await??;
//!
//! // Send a message
//! agent.send(DeliverMessage {
//!     message: proto::AclMessage { .. },
//! }).await?;
//! ```

mod agent_actor;
mod messages;
mod registry;
mod supervisor;

pub use agent_actor::AgentActor;
pub use messages::*;
pub use registry::ActorRegistry;
pub use supervisor::Supervisor;

// Re-export NetworkActor from network module
pub use crate::network::NetworkActor;
