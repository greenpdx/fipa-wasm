// wasm/host.rs - Host State for WASM Runtime

use std::collections::{HashMap, VecDeque};
use crate::proto;

/// Host state accessible from WASM through host functions
pub struct HostState {
    /// Agent ID
    pub agent_id: proto::AgentId,

    /// Agent capabilities
    pub capabilities: proto::AgentCapabilities,

    /// Current node ID
    pub node_id: String,

    /// Incoming message mailbox
    pub mailbox: VecDeque<proto::AclMessage>,

    /// Outgoing messages to send
    pub outbox: VecDeque<proto::AclMessage>,

    /// Persistent storage
    pub storage: HashMap<String, Vec<u8>>,

    /// Storage usage in bytes
    pub storage_usage: u64,

    /// Registered services
    pub services: Vec<proto::ServiceDescription>,

    /// Active timers (timer_id -> deadline_ms)
    pub timers: HashMap<u64, u64>,

    /// Next timer ID
    pub next_timer_id: u64,

    /// Fired timer IDs
    pub fired_timers: Vec<u64>,

    /// Shutdown requested flag
    pub shutdown_requested: bool,

    /// Is agent currently migrating
    pub is_migrating: bool,

    /// Migration history
    pub migration_history: Vec<String>,

    /// Statistics
    pub messages_sent: u64,
    pub messages_received: u64,
    pub log_count: u64,
}

impl HostState {
    /// Create new host state
    pub fn new(capabilities: proto::AgentCapabilities) -> Self {
        Self {
            agent_id: proto::AgentId {
                name: String::new(),
                addresses: vec![],
                resolvers: vec![],
            },
            capabilities,
            node_id: String::new(),
            mailbox: VecDeque::new(),
            outbox: VecDeque::new(),
            storage: HashMap::new(),
            storage_usage: 0,
            services: vec![],
            timers: HashMap::new(),
            next_timer_id: 1,
            fired_timers: vec![],
            shutdown_requested: false,
            is_migrating: false,
            migration_history: vec![],
            messages_sent: 0,
            messages_received: 0,
            log_count: 0,
        }
    }

    /// Set agent ID
    pub fn set_agent_id(&mut self, id: proto::AgentId) {
        self.agent_id = id;
    }

    /// Set node ID
    pub fn set_node_id(&mut self, node_id: String) {
        self.node_id = node_id;
    }

    /// Queue a message for the agent
    pub fn queue_message(&mut self, msg: proto::AclMessage) {
        self.mailbox.push_back(msg);
        self.messages_received += 1;
    }

    /// Get next outgoing message
    pub fn pop_outgoing(&mut self) -> Option<proto::AclMessage> {
        self.outbox.pop_front()
    }

    /// Store data
    pub fn store(&mut self, key: String, value: Vec<u8>) -> Result<(), StorageError> {
        let new_usage = self.storage_usage + value.len() as u64
            - self.storage.get(&key).map(|v| v.len() as u64).unwrap_or(0);

        if new_usage > self.capabilities.storage_quota_bytes {
            return Err(StorageError::QuotaExceeded);
        }

        self.storage_usage = new_usage;
        self.storage.insert(key, value);
        Ok(())
    }

    /// Load data
    pub fn load(&self, key: &str) -> Option<&Vec<u8>> {
        self.storage.get(key)
    }

    /// Delete data
    pub fn delete(&mut self, key: &str) -> bool {
        if let Some(value) = self.storage.remove(key) {
            self.storage_usage -= value.len() as u64;
            true
        } else {
            false
        }
    }

    /// Schedule a timer
    pub fn schedule_timer(&mut self, delay_ms: u64) -> u64 {
        let timer_id = self.next_timer_id;
        self.next_timer_id += 1;

        let deadline = chrono::Utc::now().timestamp_millis() as u64 + delay_ms;
        self.timers.insert(timer_id, deadline);

        timer_id
    }

    /// Cancel a timer
    pub fn cancel_timer(&mut self, timer_id: u64) -> bool {
        self.timers.remove(&timer_id).is_some()
    }

    /// Check and collect fired timers
    pub fn check_timers(&mut self) {
        let now = chrono::Utc::now().timestamp_millis() as u64;

        let fired: Vec<u64> = self.timers
            .iter()
            .filter(|(_, deadline)| now >= **deadline)
            .map(|(id, _)| *id)
            .collect();

        for id in &fired {
            self.timers.remove(id);
            self.fired_timers.push(*id);
        }
    }

    /// Get and clear fired timers
    pub fn take_fired_timers(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.fired_timers)
    }

    /// Register a service
    pub fn register_service(&mut self, service: proto::ServiceDescription) {
        // Remove existing service with same name
        self.services.retain(|s| s.name != service.name);
        self.services.push(service);
    }

    /// Deregister a service
    pub fn deregister_service(&mut self, name: &str) -> bool {
        let len_before = self.services.len();
        self.services.retain(|s| s.name != name);
        self.services.len() < len_before
    }
}

/// Storage error types
#[derive(Debug, Clone, thiserror::Error)]
pub enum StorageError {
    #[error("Storage quota exceeded")]
    QuotaExceeded,

    #[error("Key not found: {0}")]
    NotFound(String),

    #[error("I/O error: {0}")]
    IoError(String),
}
