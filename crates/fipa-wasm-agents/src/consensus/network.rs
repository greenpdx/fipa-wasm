// consensus/network.rs - Raft Network Layer

use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::error::{InstallSnapshotError, NetworkError, RPCError, RaftError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork as RaftNetworkTrait, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use parking_lot::RwLock;
use tonic::transport::Channel;

use super::types::{NodeId, NodeInfo, TypeConfig};
use crate::observability::record_consensus_election;
use crate::proto::consensus_service_client::ConsensusServiceClient;
use crate::proto::{self as pb};

/// Raft network implementation using gRPC
pub struct RaftNetwork {
    /// Known node addresses
    nodes: RwLock<BTreeMap<NodeId, NodeInfo>>,

    /// Cached gRPC connections
    connections: RwLock<BTreeMap<NodeId, ConsensusServiceClient<Channel>>>,
}

impl RaftNetwork {
    /// Create a new network
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(BTreeMap::new()),
            connections: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a node
    pub fn add_node(&self, id: NodeId, info: NodeInfo) {
        self.nodes.write().insert(id, info);
    }

    /// Remove a node
    pub fn remove_node(&self, id: NodeId) {
        self.nodes.write().remove(&id);
        self.connections.write().remove(&id);
    }

    /// Get connection to a node
    async fn get_connection(
        &self,
        target: NodeId,
    ) -> Result<ConsensusServiceClient<Channel>, Unreachable> {
        // Check cache first
        {
            let conns = self.connections.read();
            if let Some(client) = conns.get(&target) {
                return Ok(client.clone());
            }
        }

        // Get node info
        let info = {
            let nodes = self.nodes.read();
            nodes.get(&target).cloned()
        };

        let info = info.ok_or_else(|| {
            Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Node {} not found", target),
            ))
        })?;

        // Connect
        let endpoint = format!("http://{}", info.grpc_addr);
        let channel = Channel::from_shared(endpoint)
            .map_err(|e| {
                Unreachable::new(&std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    e,
                ))
            })?
            .connect()
            .await
            .map_err(|e| {
                Unreachable::new(&std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    e,
                ))
            })?;

        let client = ConsensusServiceClient::new(channel);

        // Cache connection
        self.connections.write().insert(target, client.clone());

        Ok(client)
    }
}

impl Default for RaftNetwork {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-connection network handler
pub struct RaftConnection {
    network: Arc<RaftNetwork>,
    target: NodeId,
    #[allow(dead_code)]
    target_node: NodeInfo,
}

impl RaftNetworkFactory<TypeConfig> for Arc<RaftNetwork> {
    type Network = RaftConnection;

    async fn new_client(&mut self, target: NodeId, node: &NodeInfo) -> Self::Network {
        self.add_node(target, node.clone());
        RaftConnection {
            network: Arc::clone(self),
            target,
            target_node: node.clone(),
        }
    }
}

impl RaftNetworkTrait<TypeConfig> for RaftConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, NodeInfo, RaftError<NodeId>>> {
        let mut client = self.network.get_connection(self.target).await
            .map_err(RPCError::Unreachable)?;

        // Serialize the request
        let data = bincode::serde::encode_to_vec(&req, bincode::config::standard())
            .map_err(|e| RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::InvalidData, e
            ))))?;

        let pb_req = pb::AppendRequest {
            term: req.vote.leader_id().term,
            leader_id: req.vote.leader_id().node_id,
            entries: data,
        };

        let response = client.append(pb_req).await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionReset, e
            ))))?;

        let inner = response.into_inner();

        // Deserialize the response
        if inner.data.is_empty() {
            return Err(RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Empty response"
            ))));
        }

        let (resp, _): (AppendEntriesResponse<NodeId>, _) = bincode::serde::decode_from_slice(
            &inner.data, bincode::config::standard()
        ).map_err(|e| RPCError::Network(NetworkError::new(&std::io::Error::new(
            std::io::ErrorKind::InvalidData, e
        ))))?;

        Ok(resp)
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<NodeId>,
        RPCError<NodeId, NodeInfo, RaftError<NodeId, InstallSnapshotError>>,
    > {
        let mut client = self.network.get_connection(self.target).await
            .map_err(RPCError::Unreachable)?;

        // Serialize the request
        let data = bincode::serde::encode_to_vec(&req, bincode::config::standard())
            .map_err(|e| RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::InvalidData, e
            ))))?;

        let pb_req = pb::SnapshotRequest {
            term: req.vote.leader_id().term,
            leader_id: req.vote.leader_id().node_id,
            snapshot_id: req.meta.snapshot_id.clone(),
            data,
        };

        let response = client.snapshot(pb_req).await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionReset, e
            ))))?;

        let inner = response.into_inner();

        // Deserialize the response
        if inner.data.is_empty() {
            return Err(RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Empty response"
            ))));
        }

        let (resp, _): (InstallSnapshotResponse<NodeId>, _) = bincode::serde::decode_from_slice(
            &inner.data, bincode::config::standard()
        ).map_err(|e| RPCError::Network(NetworkError::new(&std::io::Error::new(
            std::io::ErrorKind::InvalidData, e
        ))))?;

        Ok(resp)
    }

    async fn vote(
        &mut self,
        req: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, NodeInfo, RaftError<NodeId>>> {
        let mut client = self.network.get_connection(self.target).await
            .map_err(RPCError::Unreachable)?;

        // Create protobuf request
        let pb_req = pb::VoteRequest {
            term: req.vote.leader_id().term,
            candidate_id: req.vote.leader_id().node_id,
            last_log_id: req.last_log_id.map(|id| pb::LogId {
                term: id.leader_id.term,
                index: id.index,
            }),
        };

        let response = client.vote(pb_req).await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
                std::io::ErrorKind::ConnectionReset, e
            ))))?;

        let inner = response.into_inner();

        // Deserialize the response
        if inner.data.is_empty() {
            return Err(RPCError::Network(NetworkError::new(&std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Empty response"
            ))));
        }

        let (resp, _): (VoteResponse<NodeId>, _) = bincode::serde::decode_from_slice(
            &inner.data, bincode::config::standard()
        ).map_err(|e| RPCError::Network(NetworkError::new(&std::io::Error::new(
            std::io::ErrorKind::InvalidData, e
        ))))?;

        // Record election metric
        if resp.vote_granted {
            record_consensus_election(req.vote.leader_id().term, false);
        }

        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_creation() {
        let network = RaftNetwork::new();
        network.add_node(1, NodeInfo {
            grpc_addr: "127.0.0.1:9000".into(),
            peer_id: None,
            name: Some("node-1".into()),
        });

        let nodes = network.nodes.read();
        assert!(nodes.contains_key(&1));
    }
}
