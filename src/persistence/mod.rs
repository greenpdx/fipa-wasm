// persistence/mod.rs - FIPA Persistence Framework
//
//! Persistence and Recovery Framework
//!
//! This module provides comprehensive persistence features:
//!
//! - **Agent Snapshots**: Capture and restore agent state
//! - **Platform State**: Persist platform-wide configuration
//! - **Conversation Recovery**: Resume in-progress conversations
//! - **Service Registry Persistence**: Maintain registrations across restarts
//!
//! # Architecture
//!
//! ```text
//! +------------------+     +------------------+     +------------------+
//! | PersistenceManager|---->|  SnapshotStore   |---->|  StorageBackend  |
//! | (coordination)   |     |  (serialization) |     |  (file/db/mem)   |
//! +------------------+     +------------------+     +------------------+
//!         |                        |                        |
//!         v                        v                        v
//! +------------------+     +------------------+     +------------------+
//! |  RecoveryEngine  |     |  AgentSnapshot   |     |   Stored Data    |
//! +------------------+     +------------------+     +------------------+
//! ```
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::persistence::{PersistenceManager, PersistenceConfig};
//!
//! let config = PersistenceConfig::default();
//! let manager = PersistenceManager::new(config).await?;
//!
//! // Take a snapshot
//! manager.snapshot_agent("my-agent").await?;
//!
//! // Recover platform state
//! let state = manager.recover().await?;
//! ```

pub mod recovery;
pub mod snapshot;
pub mod storage;

pub use recovery::{RecoveryEngine, RecoveryError, RecoveryResult, RecoveryState, RecoveredAgent, RecoveredPlatform};
pub use snapshot::{
    AgentSnapshot, ConversationSnapshot, PlatformSnapshot, ServiceSnapshot, SnapshotError,
    SnapshotId, SnapshotMetadata,
};
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, info, warn};

/// Persistence errors
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),

    #[error("Recovery error: {0}")]
    Recovery(#[from] RecoveryError),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Persistence configuration
#[derive(Debug, Clone)]
pub struct PersistenceConfig {
    /// Enable persistence
    pub enabled: bool,

    /// Storage directory for file-based persistence
    pub storage_path: PathBuf,

    /// Automatic snapshot interval (0 = disabled)
    pub snapshot_interval_secs: u64,

    /// Maximum snapshots to keep per agent
    pub max_snapshots_per_agent: usize,

    /// Enable conversation persistence
    pub persist_conversations: bool,

    /// Enable service registry persistence
    pub persist_services: bool,

    /// Compress snapshots
    pub compress_snapshots: bool,

    /// Storage backend type
    pub backend: StorageBackend,
}

/// Storage backend types
#[derive(Debug, Clone, PartialEq)]
pub enum StorageBackend {
    /// File-based storage
    File,
    /// In-memory storage (for testing)
    Memory,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_path: PathBuf::from("./data/persistence"),
            snapshot_interval_secs: 300, // 5 minutes
            max_snapshots_per_agent: 5,
            persist_conversations: true,
            persist_services: true,
            compress_snapshots: false,
            backend: StorageBackend::File,
        }
    }
}

impl PersistenceConfig {
    /// Create a memory-only configuration (for testing)
    pub fn memory() -> Self {
        Self {
            enabled: true,
            storage_path: PathBuf::from("/tmp/fipa-test"),
            snapshot_interval_secs: 0,
            max_snapshots_per_agent: 10,
            persist_conversations: true,
            persist_services: true,
            compress_snapshots: false,
            backend: StorageBackend::Memory,
        }
    }

    /// Disable persistence
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Persistence manager - main entry point for persistence operations
pub struct PersistenceManager {
    /// Configuration
    config: PersistenceConfig,

    /// Storage backend
    storage: Arc<RwLock<Box<dyn Storage>>>,

    /// Recovery engine
    recovery_engine: Arc<RwLock<RecoveryEngine>>,

    /// Snapshot counter
    snapshot_counter: Arc<RwLock<u64>>,

    /// Background task handle
    background_task: Option<tokio::task::JoinHandle<()>>,
}

impl PersistenceManager {
    /// Create a new persistence manager
    pub async fn new(config: PersistenceConfig) -> Result<Self, PersistenceError> {
        let storage: Box<dyn Storage> = match config.backend {
            StorageBackend::File => {
                // Ensure storage directory exists
                if config.enabled {
                    tokio::fs::create_dir_all(&config.storage_path).await?;
                }
                Box::new(FileStorage::new(config.storage_path.clone()))
            }
            StorageBackend::Memory => Box::new(MemoryStorage::new()),
        };

        let recovery_engine = RecoveryEngine::new();

        Ok(Self {
            config,
            storage: Arc::new(RwLock::new(storage)),
            recovery_engine: Arc::new(RwLock::new(recovery_engine)),
            snapshot_counter: Arc::new(RwLock::new(0)),
            background_task: None,
        })
    }

    /// Start background snapshot task
    pub fn start_background_snapshots(&mut self, agents_provider: Arc<dyn AgentProvider>) {
        if !self.config.enabled || self.config.snapshot_interval_secs == 0 {
            return;
        }

        let interval_secs = self.config.snapshot_interval_secs;
        let storage = self.storage.clone();
        let counter = self.snapshot_counter.clone();
        let max_snapshots = self.config.max_snapshots_per_agent;

        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));

            loop {
                ticker.tick().await;

                // Get all agent names
                let agent_names = agents_provider.list_agents().await;

                for agent_name in agent_names {
                    if let Some(state) = agents_provider.get_agent_state(&agent_name).await {
                        // Create snapshot
                        let mut count = counter.write().await;
                        *count += 1;
                        let snapshot_id = SnapshotId::new(*count);

                        let snapshot = AgentSnapshot {
                            id: snapshot_id.clone(),
                            agent_name: agent_name.clone(),
                            metadata: SnapshotMetadata::now(),
                            wasm_state: state,
                            conversations: vec![],
                            pending_messages: vec![],
                        };

                        // Store snapshot
                        let mut storage = storage.write().await;
                        if let Err(e) = storage.store_agent_snapshot(&snapshot).await {
                            warn!("Failed to snapshot agent '{}': {}", agent_name, e);
                        } else {
                            debug!("Snapshot created for agent '{}'", agent_name);
                        }

                        // Clean up old snapshots
                        if let Err(e) = storage
                            .cleanup_agent_snapshots(&agent_name, max_snapshots)
                            .await
                        {
                            warn!(
                                "Failed to cleanup snapshots for '{}': {}",
                                agent_name, e
                            );
                        }
                    }
                }
            }
        });

        self.background_task = Some(handle);
        info!(
            "Started background snapshots (interval: {}s)",
            interval_secs
        );
    }

    /// Stop background snapshot task
    pub fn stop_background_snapshots(&mut self) {
        if let Some(handle) = self.background_task.take() {
            handle.abort();
            info!("Stopped background snapshots");
        }
    }

    /// Take a snapshot of a single agent
    pub async fn snapshot_agent(
        &self,
        agent_name: &str,
        state: Vec<u8>,
        conversations: Vec<ConversationSnapshot>,
    ) -> Result<SnapshotId, PersistenceError> {
        if !self.config.enabled {
            return Ok(SnapshotId::disabled());
        }

        let mut counter = self.snapshot_counter.write().await;
        *counter += 1;
        let snapshot_id = SnapshotId::new(*counter);

        let snapshot = AgentSnapshot {
            id: snapshot_id.clone(),
            agent_name: agent_name.to_string(),
            metadata: SnapshotMetadata::now(),
            wasm_state: state,
            conversations,
            pending_messages: vec![],
        };

        let mut storage = self.storage.write().await;
        storage.store_agent_snapshot(&snapshot).await?;

        debug!("Created snapshot {} for agent '{}'", snapshot_id, agent_name);

        // Cleanup old snapshots
        storage
            .cleanup_agent_snapshots(agent_name, self.config.max_snapshots_per_agent)
            .await?;

        Ok(snapshot_id)
    }

    /// Get the latest snapshot for an agent
    pub async fn get_latest_snapshot(
        &self,
        agent_name: &str,
    ) -> Result<Option<AgentSnapshot>, PersistenceError> {
        if !self.config.enabled {
            return Ok(None);
        }

        let storage = self.storage.read().await;
        Ok(storage.get_latest_agent_snapshot(agent_name).await?)
    }

    /// Get a specific snapshot by ID
    pub async fn get_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<AgentSnapshot>, PersistenceError> {
        if !self.config.enabled {
            return Ok(None);
        }

        let storage = self.storage.read().await;
        Ok(storage.get_agent_snapshot(snapshot_id).await?)
    }

    /// List all snapshots for an agent
    pub async fn list_snapshots(
        &self,
        agent_name: &str,
    ) -> Result<Vec<SnapshotMetadata>, PersistenceError> {
        if !self.config.enabled {
            return Ok(vec![]);
        }

        let storage = self.storage.read().await;
        Ok(storage.list_agent_snapshots(agent_name).await?)
    }

    /// Delete a snapshot
    pub async fn delete_snapshot(&self, snapshot_id: &SnapshotId) -> Result<bool, PersistenceError> {
        if !self.config.enabled {
            return Ok(false);
        }

        let mut storage = self.storage.write().await;
        Ok(storage.delete_agent_snapshot(snapshot_id).await?)
    }

    /// Save platform state
    pub async fn save_platform_state(
        &self,
        state: &PlatformSnapshot,
    ) -> Result<(), PersistenceError> {
        if !self.config.enabled {
            return Ok(());
        }

        let mut storage = self.storage.write().await;
        storage.store_platform_snapshot(state).await?;

        info!("Saved platform state");
        Ok(())
    }

    /// Load platform state
    pub async fn load_platform_state(&self) -> Result<Option<PlatformSnapshot>, PersistenceError> {
        if !self.config.enabled {
            return Ok(None);
        }

        let storage = self.storage.read().await;
        Ok(storage.get_platform_snapshot().await?)
    }

    /// Recover platform from persisted state
    pub async fn recover(&self) -> Result<RecoveryState, PersistenceError> {
        if !self.config.enabled {
            return Ok(RecoveryState::empty());
        }

        info!("Starting platform recovery...");

        let storage = self.storage.read().await;
        let mut engine = self.recovery_engine.write().await;

        // Load platform state
        if let Some(platform) = storage.get_platform_snapshot().await? {
            engine.set_platform_state(platform);
        }

        // Load agent snapshots
        let agent_names = storage.list_agents().await?;
        for agent_name in agent_names {
            if let Some(snapshot) = storage.get_latest_agent_snapshot(&agent_name).await? {
                engine.add_agent_snapshot(snapshot);
            }
        }

        let state = engine.build_recovery_state()?;

        info!(
            "Recovery complete: {} agents, {} services",
            state.agents.len(),
            state.services.len()
        );

        Ok(state)
    }

    /// Save service registrations
    pub async fn save_services(
        &self,
        services: Vec<ServiceSnapshot>,
    ) -> Result<(), PersistenceError> {
        if !self.config.enabled || !self.config.persist_services {
            return Ok(());
        }

        let mut storage = self.storage.write().await;
        storage.store_services(&services).await?;

        debug!("Saved {} service registrations", services.len());
        Ok(())
    }

    /// Load service registrations
    pub async fn load_services(&self) -> Result<Vec<ServiceSnapshot>, PersistenceError> {
        if !self.config.enabled || !self.config.persist_services {
            return Ok(vec![]);
        }

        let storage = self.storage.read().await;
        Ok(storage.get_services().await?)
    }

    /// Get storage statistics
    pub async fn stats(&self) -> PersistenceStats {
        let storage = self.storage.read().await;
        storage.stats().await
    }

    /// Check if persistence is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the configuration
    pub fn config(&self) -> &PersistenceConfig {
        &self.config
    }
}

impl Drop for PersistenceManager {
    fn drop(&mut self) {
        if let Some(handle) = self.background_task.take() {
            handle.abort();
        }
    }
}

/// Trait for providing agent state (used by background snapshots)
#[async_trait::async_trait]
pub trait AgentProvider: Send + Sync {
    /// List all agent names
    async fn list_agents(&self) -> Vec<String>;

    /// Get agent state for snapshotting
    async fn get_agent_state(&self, agent_name: &str) -> Option<Vec<u8>>;
}

/// Persistence statistics
#[derive(Debug, Clone, Default)]
pub struct PersistenceStats {
    /// Total number of snapshots
    pub total_snapshots: usize,

    /// Total storage size in bytes
    pub storage_bytes: u64,

    /// Number of agents with snapshots
    pub agents_with_snapshots: usize,

    /// Number of stored services
    pub stored_services: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_persistence_config_default() {
        let config = PersistenceConfig::default();
        assert!(config.enabled);
        assert_eq!(config.snapshot_interval_secs, 300);
        assert_eq!(config.max_snapshots_per_agent, 5);
    }

    #[tokio::test]
    async fn test_persistence_config_memory() {
        let config = PersistenceConfig::memory();
        assert!(config.enabled);
        assert_eq!(config.backend, StorageBackend::Memory);
    }

    #[tokio::test]
    async fn test_persistence_manager_creation() {
        let config = PersistenceConfig::memory();
        let manager = PersistenceManager::new(config).await.unwrap();

        assert!(manager.is_enabled());
    }

    #[tokio::test]
    async fn test_snapshot_agent() {
        let config = PersistenceConfig::memory();
        let manager = PersistenceManager::new(config).await.unwrap();

        let snapshot_id = manager
            .snapshot_agent("test-agent", vec![1, 2, 3], vec![])
            .await
            .unwrap();

        assert!(!snapshot_id.is_disabled());

        // Retrieve snapshot
        let snapshot = manager.get_latest_snapshot("test-agent").await.unwrap();
        assert!(snapshot.is_some());

        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.agent_name, "test-agent");
        assert_eq!(snapshot.wasm_state, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_disabled_persistence() {
        let config = PersistenceConfig::disabled();
        let manager = PersistenceManager::new(config).await.unwrap();

        assert!(!manager.is_enabled());

        let snapshot_id = manager
            .snapshot_agent("test", vec![], vec![])
            .await
            .unwrap();

        assert!(snapshot_id.is_disabled());
    }
}
