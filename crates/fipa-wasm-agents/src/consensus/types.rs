// consensus/types.rs - Raft Type Configuration

use serde::{Deserialize, Serialize};
use std::io::Cursor;

/// Node identifier in the Raft cluster
pub type NodeId = u64;

/// Node address information
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// gRPC address for consensus messages
    pub grpc_addr: String,

    /// libp2p peer ID (if available)
    pub peer_id: Option<String>,

    /// Human-readable node name
    pub name: Option<String>,
}

impl std::fmt::Display for NodeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.grpc_addr)
    }
}

/// Configuration for Raft consensus
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RaftConfig {
    /// Heartbeat interval in milliseconds
    pub heartbeat_interval_ms: u64,

    /// Minimum election timeout in milliseconds
    pub election_timeout_min_ms: u64,

    /// Maximum election timeout in milliseconds
    pub election_timeout_max_ms: u64,

    /// Maximum entries per append request
    pub max_payload_entries: u64,

    /// Snapshot replication chunk size
    pub snapshot_chunk_size: u64,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_ms: 150,
            election_timeout_min_ms: 300,
            election_timeout_max_ms: 600,
            max_payload_entries: 300,
            snapshot_chunk_size: 1024 * 1024, // 1MB chunks
        }
    }
}

/// Request type for the state machine
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub data: Vec<u8>,
}

/// Response type from the state machine
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub data: Option<Vec<u8>>,
}

/// Openraft type configuration for FIPA
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct TypeConfig;

impl std::fmt::Display for TypeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FipaTypeConfig")
    }
}

impl openraft::RaftTypeConfig for TypeConfig {
    type D = Request;
    type R = Response;
    type Node = NodeInfo;
    type NodeId = NodeId;
    type Entry = openraft::Entry<TypeConfig>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = openraft::TokioRuntime;
    type Responder = openraft::impls::OneshotResponder<TypeConfig>;
}
