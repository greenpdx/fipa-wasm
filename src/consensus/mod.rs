// consensus/mod.rs - Raft Consensus Module

//! Raft consensus implementation using openraft.
//!
//! This module provides distributed consensus for:
//! - Agent directory (agent -> node mapping)
//! - Service registry (service discovery)
//! - Cluster membership
//!
//! Uses openraft with sled for persistent storage.

mod network;
mod state;
mod storage;
mod types;

pub use network::RaftNetwork;
pub use state::{AgentLocation, ClusterState, ServiceEntry, StateRequest, StateResponse};
pub use storage::RaftStore;
pub use types::{NodeId, NodeInfo, RaftConfig, TypeConfig};

use openraft::storage::Adaptor;
use openraft::Raft;
use std::sync::Arc;

/// The Raft consensus instance type
pub type RaftInstance = Raft<TypeConfig>;

/// The Raft log store type (using Adaptor)
pub type LogStore = Adaptor<TypeConfig, Arc<RaftStore>>;

/// The Raft state machine type (using Adaptor)
pub type StateMachine = Adaptor<TypeConfig, Arc<RaftStore>>;

/// Create a new Raft instance with the given configuration
pub async fn create_raft(
    node_id: NodeId,
    config: RaftConfig,
    store: Arc<RaftStore>,
    network: Arc<RaftNetwork>,
) -> Result<RaftInstance, openraft::error::Fatal<NodeId>> {
    let raft_config = Arc::new(
        openraft::Config {
            cluster_name: "fipa-cluster".into(),
            heartbeat_interval: config.heartbeat_interval_ms,
            election_timeout_min: config.election_timeout_min_ms,
            election_timeout_max: config.election_timeout_max_ms,
            ..Default::default()
        }
    );

    // Create log store and state machine using the Adaptor
    let (log_store, state_machine) = Adaptor::new(store);

    Raft::new(node_id, raft_config, network, log_store, state_machine).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id() {
        let id: NodeId = 1;
        assert_eq!(id, 1);
    }
}
