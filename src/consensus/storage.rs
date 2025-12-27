// consensus/storage.rs - Raft Storage with Sled

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::{
    Entry, LogId, LogState, OptionalSend, RaftLogReader, RaftStorage, RaftSnapshotBuilder,
    Snapshot, SnapshotMeta, StorageError, StoredMembership, Vote,
};
use tokio::sync::RwLock;
use sled::Db;

use super::state::{ClusterState, StateRequest, StateResponse};
use super::types::{NodeId, NodeInfo, Response, TypeConfig};
use crate::observability::record_consensus_commit;

const TREE_LOGS: &str = "raft_logs";
const TREE_META: &str = "raft_meta";
const KEY_VOTE: &str = "vote";
const KEY_COMMITTED: &str = "committed";
#[allow(dead_code)]
const KEY_LAST_APPLIED: &str = "last_applied";
#[allow(dead_code)]
const KEY_MEMBERSHIP: &str = "membership";

/// Raft storage implementation using sled
pub struct RaftStore {
    /// Sled database
    db: Db,

    /// In-memory log cache for performance
    log_cache: RwLock<BTreeMap<u64, Entry<TypeConfig>>>,

    /// Current state
    state: RwLock<ClusterState>,

    /// This node's ID
    node_id: NodeId,
}

impl RaftStore {
    /// Create new storage
    pub fn new(db: Db, node_id: NodeId) -> Self {
        Self {
            db,
            log_cache: RwLock::new(BTreeMap::new()),
            state: RwLock::new(ClusterState::default()),
            node_id,
        }
    }

    /// Open storage from path
    pub fn open(path: &std::path::Path, node_id: NodeId) -> Result<Self, sled::Error> {
        let db = sled::open(path)?;
        Ok(Self::new(db, node_id))
    }

    fn logs_tree(&self) -> sled::Tree {
        self.db.open_tree(TREE_LOGS).expect("Failed to open logs tree")
    }

    fn meta_tree(&self) -> sled::Tree {
        self.db.open_tree(TREE_META).expect("Failed to open meta tree")
    }

    fn encode_log_key(index: u64) -> [u8; 8] {
        index.to_be_bytes()
    }

    /// Get the underlying database
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Query agent location (read-only)
    pub async fn get_agent(&self, fingerprint: &str) -> Option<super::state::AgentLocation> {
        self.state.read().await.agents.get(fingerprint).cloned()
    }

    /// Query services (read-only)
    pub async fn get_services(&self, service_type: &str) -> Vec<super::state::ServiceEntry> {
        self.state.read().await.services.get(service_type).cloned().unwrap_or_default()
    }
}

impl RaftLogReader<TypeConfig> for Arc<RaftStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let cache = self.log_cache.read().await;
        Ok(cache.range(range).map(|(_, e)| e.clone()).collect())
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<RaftStore> {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let state = self.state.read().await;

        let data = bincode::serde::encode_to_vec(&*state, bincode::config::standard())
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::StateMachine,
                openraft::ErrorVerb::Read,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        let snapshot_id = format!(
            "{}-{}",
            state.last_applied_log.map(|l| l.index).unwrap_or(0),
            chrono::Utc::now().timestamp()
        );

        Ok(Snapshot {
            meta: SnapshotMeta {
                last_log_id: state.last_applied_log,
                last_membership: state.last_membership.clone(),
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStorage<TypeConfig> for Arc<RaftStore> {
    type LogReader = Self;
    type SnapshotBuilder = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let cache = self.log_cache.read().await;
        let last = cache.iter().next_back().map(|(_, e)| e.log_id);

        Ok(LogState {
            last_purged_log_id: None,
            last_log_id: last,
        })
    }

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        let meta = self.meta_tree();
        let data = bincode::serde::encode_to_vec(vote, bincode::config::standard())
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Vote,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        meta.insert(KEY_VOTE, data)
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Vote,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        meta.flush_async().await
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Vote,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        let meta = self.meta_tree();

        let vote = meta
            .get(KEY_VOTE)
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Vote,
                openraft::ErrorVerb::Read,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?
            .and_then(|v| {
                bincode::serde::decode_from_slice(&v, bincode::config::standard())
                    .ok()
                    .map(|(vote, _)| vote)
            });

        Ok(vote)
    }

    async fn save_committed(&mut self, committed: Option<LogId<NodeId>>) -> Result<(), StorageError<NodeId>> {
        let meta = self.meta_tree();

        if let Some(c) = committed {
            let data = bincode::serde::encode_to_vec(&c, bincode::config::standard())
                .map_err(|e| StorageError::from_io_error(
                    openraft::ErrorSubject::Logs,
                    openraft::ErrorVerb::Write,
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))?;
            meta.insert(KEY_COMMITTED, data)
                .map_err(|e| StorageError::from_io_error(
                    openraft::ErrorSubject::Logs,
                    openraft::ErrorVerb::Write,
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))?;
        } else {
            meta.remove(KEY_COMMITTED)
                .map_err(|e| StorageError::from_io_error(
                    openraft::ErrorSubject::Logs,
                    openraft::ErrorVerb::Write,
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))?;
        }

        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        let meta = self.meta_tree();

        let committed = meta
            .get(KEY_COMMITTED)
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Logs,
                openraft::ErrorVerb::Read,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?
            .and_then(|v| {
                bincode::serde::decode_from_slice(&v, bincode::config::standard())
                    .ok()
                    .map(|(id, _)| id)
            });

        Ok(committed)
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        Arc::clone(self)
    }

    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
    {
        let logs = self.logs_tree();
        let mut cache = self.log_cache.write().await;

        for entry in entries {
            let key = RaftStore::encode_log_key(entry.log_id.index);
            let data = bincode::serde::encode_to_vec(&entry, bincode::config::standard())
                .map_err(|e| StorageError::from_io_error(
                    openraft::ErrorSubject::Logs,
                    openraft::ErrorVerb::Write,
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))?;

            logs.insert(&key, data)
                .map_err(|e| StorageError::from_io_error(
                    openraft::ErrorSubject::Logs,
                    openraft::ErrorVerb::Write,
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))?;

            cache.insert(entry.log_id.index, entry);
        }

        logs.flush_async().await
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Logs,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        Ok(())
    }

    async fn delete_conflict_logs_since(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let logs = self.logs_tree();
        let mut cache = self.log_cache.write().await;

        let to_remove: Vec<_> = cache.range(log_id.index..).map(|(k, _)| *k).collect();
        for idx in &to_remove {
            cache.remove(idx);
            let key = RaftStore::encode_log_key(*idx);
            logs.remove(&key).map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Logs,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;
        }

        Ok(())
    }

    async fn purge_logs_upto(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let logs = self.logs_tree();
        let mut cache = self.log_cache.write().await;

        let to_remove: Vec<_> = cache.range(..=log_id.index).map(|(k, _)| *k).collect();
        for idx in &to_remove {
            cache.remove(idx);
            let key = RaftStore::encode_log_key(*idx);
            logs.remove(&key).map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::Logs,
                openraft::ErrorVerb::Write,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;
        }

        Ok(())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, NodeInfo>), StorageError<NodeId>> {
        let state = self.state.read().await;
        Ok((state.last_applied_log, state.last_membership.clone()))
    }

    async fn apply_to_state_machine(
        &mut self,
        entries: &[Entry<TypeConfig>],
    ) -> Result<Vec<Response>, StorageError<NodeId>> {
        let mut responses = Vec::new();
        let mut state = self.state.write().await;

        for entry in entries {
            state.last_applied_log = Some(entry.log_id);
            record_consensus_commit(entry.log_id.index);

            match &entry.payload {
                openraft::EntryPayload::Blank => {
                    responses.push(Response { data: None });
                }
                openraft::EntryPayload::Normal(request) => {
                    let state_request: StateRequest = match bincode::serde::decode_from_slice(
                        &request.data,
                        bincode::config::standard(),
                    ) {
                        Ok((req, _)) => req,
                        Err(e) => {
                            responses.push(Response {
                                data: Some(
                                    bincode::serde::encode_to_vec(
                                        &StateResponse::Error(e.to_string()),
                                        bincode::config::standard(),
                                    ).unwrap_or_default()
                                ),
                            });
                            continue;
                        }
                    };

                    let state_response = state.apply(state_request, self.node_id);
                    let response_data = bincode::serde::encode_to_vec(
                        &state_response,
                        bincode::config::standard(),
                    ).ok();

                    responses.push(Response { data: response_data });
                }
                openraft::EntryPayload::Membership(membership) => {
                    state.last_membership = StoredMembership::new(Some(entry.log_id), membership.clone());
                    responses.push(Response { data: None });
                }
            }
        }

        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        Arc::clone(self)
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, NodeInfo>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<NodeId>> {
        let data = snapshot.into_inner();
        let (new_state, _): (ClusterState, _) = bincode::serde::decode_from_slice(
            &data,
            bincode::config::standard(),
        ).map_err(|e| StorageError::from_io_error(
            openraft::ErrorSubject::Snapshot(Some(meta.signature())),
            openraft::ErrorVerb::Read,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        ))?;

        let mut state = self.state.write().await;
        *state = new_state;
        if let Some(log_id) = meta.last_log_id {
            state.last_applied_log = Some(log_id);
        }
        state.last_membership = meta.last_membership.clone();

        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
        let state = self.state.read().await;

        if state.last_applied_log.is_none() {
            return Ok(None);
        }

        let data = bincode::serde::encode_to_vec(&*state, bincode::config::standard())
            .map_err(|e| StorageError::from_io_error(
                openraft::ErrorSubject::StateMachine,
                openraft::ErrorVerb::Read,
                std::io::Error::new(std::io::ErrorKind::Other, e),
            ))?;

        let snapshot_id = format!(
            "{}-{}",
            state.last_applied_log.map(|l| l.index).unwrap_or(0),
            chrono::Utc::now().timestamp()
        );

        Ok(Some(Snapshot {
            meta: SnapshotMeta {
                last_log_id: state.last_applied_log,
                last_membership: state.last_membership.clone(),
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_key_encoding() {
        let index = 12345u64;
        let encoded = RaftStore::encode_log_key(index);
        let decoded = u64::from_be_bytes(encoded);
        assert_eq!(index, decoded);
    }

    #[tokio::test]
    async fn test_storage_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = RaftStore::open(temp_dir.path(), 1).unwrap();
        // New database returns false for was_recovered(), only true if reopening existing
        assert!(!storage.db().was_recovered());
    }
}
