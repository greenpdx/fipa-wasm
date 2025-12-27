// network/mod.rs - Network Layer

//! Network layer using libp2p for peer-to-peer communication and gRPC services.
//!
//! This module provides:
//! - `Transport` - libp2p swarm configuration
//! - `Discovery` - mDNS and Kademlia peer discovery
//! - `NetworkActor` - Actix actor for network operations
//! - `gRPC` - tonic-based RPC services for agent messaging and consensus

mod transport;
mod discovery;
mod routing;
pub mod grpc;

pub use transport::{NetworkConfig, NetworkTransport, SwarmEvent};
pub use discovery::{DiscoveryConfig, DiscoveryService};
pub use routing::{NetworkActor, RouteMessage};
pub use grpc::{
    ConsensusServiceImpl, ConsensusState, FipaAgentServiceImpl,
    GrpcServerConfig, ServiceState, run_grpc_server,
};
