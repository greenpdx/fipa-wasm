// persistence/storage.rs - Storage Backends
//
//! Storage backend implementations
//!
//! Provides:
//! - File-based storage
//! - In-memory storage (for testing)
//! - Storage trait for custom backends

use super::snapshot::{
    AgentSnapshot, PlatformSnapshot, ServiceSnapshot, SnapshotId, SnapshotMetadata,
};
use super::PersistenceStats;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;
use tracing::debug;

/// Storage errors
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Storage full")]
    StorageFull,

    #[error("Corrupted data: {0}")]
    Corrupted(String),
}

/// Storage backend trait
#[async_trait]
pub trait Storage: Send + Sync {
    /// Store an agent snapshot
    async fn store_agent_snapshot(&mut self, snapshot: &AgentSnapshot) -> Result<(), StorageError>;

    /// Get an agent snapshot by ID
    async fn get_agent_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<AgentSnapshot>, StorageError>;

    /// Get the latest snapshot for an agent
    async fn get_latest_agent_snapshot(
        &self,
        agent_name: &str,
    ) -> Result<Option<AgentSnapshot>, StorageError>;

    /// List all snapshots for an agent
    async fn list_agent_snapshots(
        &self,
        agent_name: &str,
    ) -> Result<Vec<SnapshotMetadata>, StorageError>;

    /// Delete an agent snapshot
    async fn delete_agent_snapshot(&mut self, snapshot_id: &SnapshotId) -> Result<bool, StorageError>;

    /// Cleanup old snapshots, keeping only the most recent N
    async fn cleanup_agent_snapshots(
        &mut self,
        agent_name: &str,
        keep_count: usize,
    ) -> Result<usize, StorageError>;

    /// List all agents with snapshots
    async fn list_agents(&self) -> Result<Vec<String>, StorageError>;

    /// Store platform snapshot
    async fn store_platform_snapshot(
        &mut self,
        snapshot: &PlatformSnapshot,
    ) -> Result<(), StorageError>;

    /// Get platform snapshot
    async fn get_platform_snapshot(&self) -> Result<Option<PlatformSnapshot>, StorageError>;

    /// Store service registrations
    async fn store_services(&mut self, services: &[ServiceSnapshot]) -> Result<(), StorageError>;

    /// Get service registrations
    async fn get_services(&self) -> Result<Vec<ServiceSnapshot>, StorageError>;

    /// Get storage statistics
    async fn stats(&self) -> PersistenceStats;
}

/// File-based storage backend
pub struct FileStorage {
    /// Base path for storage
    base_path: PathBuf,
}

impl FileStorage {
    /// Create a new file storage
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Get the agents directory
    fn agents_dir(&self) -> PathBuf {
        self.base_path.join("agents")
    }

    /// Get the directory for a specific agent
    fn agent_dir(&self, agent_name: &str) -> PathBuf {
        self.agents_dir().join(sanitize_filename(agent_name))
    }

    /// Get the platform snapshot file path
    fn platform_file(&self) -> PathBuf {
        self.base_path.join("platform.json")
    }

    /// Get the services file path
    fn services_file(&self) -> PathBuf {
        self.base_path.join("services.json")
    }

    /// Ensure a directory exists
    async fn ensure_dir(&self, path: &PathBuf) -> Result<(), StorageError> {
        if !path.exists() {
            fs::create_dir_all(path).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Storage for FileStorage {
    async fn store_agent_snapshot(&mut self, snapshot: &AgentSnapshot) -> Result<(), StorageError> {
        let agent_dir = self.agent_dir(&snapshot.agent_name);
        self.ensure_dir(&agent_dir).await?;

        let filename = format!("{}.json", snapshot.id.to_filename());
        let filepath = agent_dir.join(&filename);

        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        fs::write(&filepath, &bytes).await?;

        debug!(
            "Stored snapshot {} for agent '{}' at {:?}",
            snapshot.id, snapshot.agent_name, filepath
        );

        Ok(())
    }

    async fn get_agent_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<AgentSnapshot>, StorageError> {
        // Search through all agent directories
        let agents_dir = self.agents_dir();
        if !agents_dir.exists() {
            return Ok(None);
        }

        let filename = format!("{}.json", snapshot_id.to_filename());

        let mut entries = fs::read_dir(&agents_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let filepath = entry.path().join(&filename);
                if filepath.exists() {
                    let bytes = fs::read(&filepath).await?;
                    let snapshot: AgentSnapshot = serde_json::from_slice(&bytes)
                        .map_err(|e| StorageError::Deserialization(e.to_string()))?;
                    return Ok(Some(snapshot));
                }
            }
        }

        Ok(None)
    }

    async fn get_latest_agent_snapshot(
        &self,
        agent_name: &str,
    ) -> Result<Option<AgentSnapshot>, StorageError> {
        let agent_dir = self.agent_dir(agent_name);
        if !agent_dir.exists() {
            return Ok(None);
        }

        // Find the most recent snapshot file
        let mut latest_file: Option<(u64, PathBuf)> = None;

        let mut entries = fs::read_dir(&agent_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(id) = SnapshotId::from_filename(filename) {
                        if latest_file.is_none() || id.timestamp > latest_file.as_ref().unwrap().0 {
                            latest_file = Some((id.timestamp, path));
                        }
                    }
                }
            }
        }

        if let Some((_, filepath)) = latest_file {
            let bytes = fs::read(&filepath).await?;
            let snapshot: AgentSnapshot = serde_json::from_slice(&bytes)
                .map_err(|e| StorageError::Deserialization(e.to_string()))?;
            Ok(Some(snapshot))
        } else {
            Ok(None)
        }
    }

    async fn list_agent_snapshots(
        &self,
        agent_name: &str,
    ) -> Result<Vec<SnapshotMetadata>, StorageError> {
        let agent_dir = self.agent_dir(agent_name);
        if !agent_dir.exists() {
            return Ok(vec![]);
        }

        let mut snapshots = vec![];

        let mut entries = fs::read_dir(&agent_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(bytes) = fs::read(&path).await {
                    if let Ok(snapshot) = serde_json::from_slice::<AgentSnapshot>(&bytes) {
                        snapshots.push(snapshot.metadata);
                    }
                }
            }
        }

        // Sort by timestamp descending
        snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(snapshots)
    }

    async fn delete_agent_snapshot(&mut self, snapshot_id: &SnapshotId) -> Result<bool, StorageError> {
        let agents_dir = self.agents_dir();
        if !agents_dir.exists() {
            return Ok(false);
        }

        let filename = format!("{}.json", snapshot_id.to_filename());

        let mut entries = fs::read_dir(&agents_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let filepath = entry.path().join(&filename);
                if filepath.exists() {
                    fs::remove_file(&filepath).await?;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    async fn cleanup_agent_snapshots(
        &mut self,
        agent_name: &str,
        keep_count: usize,
    ) -> Result<usize, StorageError> {
        let snapshots = self.list_agent_snapshots(agent_name).await?;

        if snapshots.len() <= keep_count {
            return Ok(0);
        }

        let to_delete = &snapshots[keep_count..];
        let mut deleted = 0;

        for metadata in to_delete {
            if self.delete_agent_snapshot(&metadata.snapshot_id).await? {
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    async fn list_agents(&self) -> Result<Vec<String>, StorageError> {
        let agents_dir = self.agents_dir();
        if !agents_dir.exists() {
            return Ok(vec![]);
        }

        let mut agents = vec![];

        let mut entries = fs::read_dir(&agents_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    agents.push(name.to_string());
                }
            }
        }

        Ok(agents)
    }

    async fn store_platform_snapshot(
        &mut self,
        snapshot: &PlatformSnapshot,
    ) -> Result<(), StorageError> {
        self.ensure_dir(&self.base_path).await?;

        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        fs::write(self.platform_file(), &bytes).await?;

        Ok(())
    }

    async fn get_platform_snapshot(&self) -> Result<Option<PlatformSnapshot>, StorageError> {
        let filepath = self.platform_file();
        if !filepath.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&filepath).await?;
        let snapshot: PlatformSnapshot = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::Deserialization(e.to_string()))?;

        Ok(Some(snapshot))
    }

    async fn store_services(&mut self, services: &[ServiceSnapshot]) -> Result<(), StorageError> {
        self.ensure_dir(&self.base_path).await?;

        let bytes = serde_json::to_vec_pretty(services)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        fs::write(self.services_file(), &bytes).await?;

        Ok(())
    }

    async fn get_services(&self) -> Result<Vec<ServiceSnapshot>, StorageError> {
        let filepath = self.services_file();
        if !filepath.exists() {
            return Ok(vec![]);
        }

        let bytes = fs::read(&filepath).await?;
        let services: Vec<ServiceSnapshot> = serde_json::from_slice(&bytes)
            .map_err(|e| StorageError::Deserialization(e.to_string()))?;

        Ok(services)
    }

    async fn stats(&self) -> PersistenceStats {
        let mut stats = PersistenceStats::default();

        // Count agents
        if let Ok(agents) = self.list_agents().await {
            stats.agents_with_snapshots = agents.len();

            // Count snapshots per agent
            for agent in &agents {
                if let Ok(snapshots) = self.list_agent_snapshots(agent).await {
                    stats.total_snapshots += snapshots.len();
                }
            }
        }

        // Count services
        if let Ok(services) = self.get_services().await {
            stats.stored_services = services.len();
        }

        // Calculate storage size (approximate)
        if let Ok(metadata) = std::fs::metadata(&self.base_path) {
            stats.storage_bytes = metadata.len();
        }

        stats
    }
}

/// In-memory storage backend (for testing)
pub struct MemoryStorage {
    /// Agent snapshots
    agent_snapshots: HashMap<String, Vec<AgentSnapshot>>,

    /// Platform snapshot
    platform_snapshot: Option<PlatformSnapshot>,

    /// Services
    services: Vec<ServiceSnapshot>,
}

impl MemoryStorage {
    /// Create a new memory storage
    pub fn new() -> Self {
        Self {
            agent_snapshots: HashMap::new(),
            platform_snapshot: None,
            services: vec![],
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn store_agent_snapshot(&mut self, snapshot: &AgentSnapshot) -> Result<(), StorageError> {
        let snapshots = self
            .agent_snapshots
            .entry(snapshot.agent_name.clone())
            .or_insert_with(Vec::new);

        snapshots.push(snapshot.clone());

        // Sort by timestamp descending
        snapshots.sort_by(|a, b| b.metadata.created_at.cmp(&a.metadata.created_at));

        Ok(())
    }

    async fn get_agent_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<AgentSnapshot>, StorageError> {
        for snapshots in self.agent_snapshots.values() {
            for snapshot in snapshots {
                if snapshot.id == *snapshot_id {
                    return Ok(Some(snapshot.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn get_latest_agent_snapshot(
        &self,
        agent_name: &str,
    ) -> Result<Option<AgentSnapshot>, StorageError> {
        if let Some(snapshots) = self.agent_snapshots.get(agent_name) {
            Ok(snapshots.first().cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_agent_snapshots(
        &self,
        agent_name: &str,
    ) -> Result<Vec<SnapshotMetadata>, StorageError> {
        if let Some(snapshots) = self.agent_snapshots.get(agent_name) {
            Ok(snapshots.iter().map(|s| s.metadata.clone()).collect())
        } else {
            Ok(vec![])
        }
    }

    async fn delete_agent_snapshot(&mut self, snapshot_id: &SnapshotId) -> Result<bool, StorageError> {
        for snapshots in self.agent_snapshots.values_mut() {
            if let Some(pos) = snapshots.iter().position(|s| s.id == *snapshot_id) {
                snapshots.remove(pos);
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn cleanup_agent_snapshots(
        &mut self,
        agent_name: &str,
        keep_count: usize,
    ) -> Result<usize, StorageError> {
        if let Some(snapshots) = self.agent_snapshots.get_mut(agent_name) {
            if snapshots.len() > keep_count {
                let removed = snapshots.len() - keep_count;
                snapshots.truncate(keep_count);
                return Ok(removed);
            }
        }
        Ok(0)
    }

    async fn list_agents(&self) -> Result<Vec<String>, StorageError> {
        Ok(self.agent_snapshots.keys().cloned().collect())
    }

    async fn store_platform_snapshot(
        &mut self,
        snapshot: &PlatformSnapshot,
    ) -> Result<(), StorageError> {
        self.platform_snapshot = Some(snapshot.clone());
        Ok(())
    }

    async fn get_platform_snapshot(&self) -> Result<Option<PlatformSnapshot>, StorageError> {
        Ok(self.platform_snapshot.clone())
    }

    async fn store_services(&mut self, services: &[ServiceSnapshot]) -> Result<(), StorageError> {
        self.services = services.to_vec();
        Ok(())
    }

    async fn get_services(&self) -> Result<Vec<ServiceSnapshot>, StorageError> {
        Ok(self.services.clone())
    }

    async fn stats(&self) -> PersistenceStats {
        let total_snapshots: usize = self.agent_snapshots.values().map(|v| v.len()).sum();

        PersistenceStats {
            total_snapshots,
            storage_bytes: 0, // Memory storage doesn't track this
            agents_with_snapshots: self.agent_snapshots.len(),
            stored_services: self.services.len(),
        }
    }
}

/// Sanitize a filename to be safe for filesystem
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_storage_basic() {
        let mut storage = MemoryStorage::new();

        let snapshot = AgentSnapshot::new("test-agent", vec![1, 2, 3]);
        storage.store_agent_snapshot(&snapshot).await.unwrap();

        let retrieved = storage
            .get_latest_agent_snapshot("test-agent")
            .await
            .unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().wasm_state, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_memory_storage_multiple_snapshots() {
        let mut storage = MemoryStorage::new();

        // Store multiple snapshots
        for i in 0..5 {
            let snapshot = AgentSnapshot::new("test-agent", vec![i]);
            storage.store_agent_snapshot(&snapshot).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        let snapshots = storage.list_agent_snapshots("test-agent").await.unwrap();
        assert_eq!(snapshots.len(), 5);
    }

    #[tokio::test]
    async fn test_memory_storage_cleanup() {
        let mut storage = MemoryStorage::new();

        // Store 10 snapshots
        for i in 0..10 {
            let snapshot = AgentSnapshot::new("test-agent", vec![i]);
            storage.store_agent_snapshot(&snapshot).await.unwrap();
        }

        // Cleanup to keep only 3
        let removed = storage
            .cleanup_agent_snapshots("test-agent", 3)
            .await
            .unwrap();
        assert_eq!(removed, 7);

        let snapshots = storage.list_agent_snapshots("test-agent").await.unwrap();
        assert_eq!(snapshots.len(), 3);
    }

    #[tokio::test]
    async fn test_memory_storage_platform() {
        let mut storage = MemoryStorage::new();

        let platform = PlatformSnapshot::new("test-platform")
            .with_node_id("node-1")
            .with_agent("agent-1");

        storage.store_platform_snapshot(&platform).await.unwrap();

        let retrieved = storage.get_platform_snapshot().await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().platform_name, "test-platform");
    }

    #[tokio::test]
    async fn test_memory_storage_services() {
        let mut storage = MemoryStorage::new();

        let services = vec![
            ServiceSnapshot::new("calc", "calc-agent"),
            ServiceSnapshot::new("weather", "weather-agent"),
        ];

        storage.store_services(&services).await.unwrap();

        let retrieved = storage.get_services().await.unwrap();
        assert_eq!(retrieved.len(), 2);
    }

    #[tokio::test]
    async fn test_memory_storage_stats() {
        let mut storage = MemoryStorage::new();

        // Store some data
        storage
            .store_agent_snapshot(&AgentSnapshot::new("agent1", vec![]))
            .await
            .unwrap();
        storage
            .store_agent_snapshot(&AgentSnapshot::new("agent2", vec![]))
            .await
            .unwrap();
        storage
            .store_services(&[ServiceSnapshot::new("svc1", "agent1")])
            .await
            .unwrap();

        let stats = storage.stats().await;
        assert_eq!(stats.total_snapshots, 2);
        assert_eq!(stats.agents_with_snapshots, 2);
        assert_eq!(stats.stored_services, 1);
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("simple"), "simple");
        assert_eq!(sanitize_filename("with-dash"), "with-dash");
        assert_eq!(sanitize_filename("with_underscore"), "with_underscore");
        assert_eq!(sanitize_filename("with space"), "with_space");
        assert_eq!(sanitize_filename("with/slash"), "with_slash");
    }
}
