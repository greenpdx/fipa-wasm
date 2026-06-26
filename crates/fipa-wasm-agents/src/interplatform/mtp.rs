// interplatform/mtp.rs - Message Transport Protocol
//
//! Message Transport Protocol (MTP) Framework
//!
//! MTPs handle the actual transport of messages between platforms.
//! Each MTP supports specific address schemes (http://, iiop://, etc.)

use super::envelope::MessageEnvelope;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// MTP errors
#[derive(Debug, Error)]
pub enum MtpError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Protocol not supported: {0}")]
    ProtocolNotSupported(String),

    #[error("MTP not active")]
    NotActive,

    #[error("Timeout")]
    Timeout,

    #[error("IO error: {0}")]
    Io(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// MTP status
#[derive(Debug, Clone, PartialEq)]
pub enum MtpStatus {
    /// MTP is inactive
    Inactive,
    /// MTP is starting up
    Starting,
    /// MTP is active and ready
    Active,
    /// MTP is shutting down
    Stopping,
    /// MTP encountered an error
    Error(String),
}

/// MTP configuration
#[derive(Debug, Clone)]
pub struct MtpConfig {
    /// Enable this MTP
    pub enabled: bool,

    /// Listen address for incoming messages
    pub listen_address: Option<String>,

    /// Connection timeout in seconds
    pub connection_timeout_secs: u64,

    /// Read timeout in seconds
    pub read_timeout_secs: u64,

    /// Write timeout in seconds
    pub write_timeout_secs: u64,

    /// Maximum message size in bytes
    pub max_message_size: usize,

    /// Additional protocol-specific settings
    pub extra: HashMap<String, String>,
}

impl Default for MtpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_address: None,
            connection_timeout_secs: 30,
            read_timeout_secs: 60,
            write_timeout_secs: 60,
            max_message_size: 10 * 1024 * 1024, // 10MB
            extra: HashMap::new(),
        }
    }
}

/// MTP delivery result
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    /// Delivery succeeded
    pub success: bool,

    /// Message ID
    pub message_id: String,

    /// Delivery timestamp
    pub delivered_at: u64,

    /// Response (if any)
    pub response: Option<MessageEnvelope>,

    /// Error message (if failed)
    pub error: Option<String>,
}

impl DeliveryResult {
    /// Create a successful result
    pub fn success(message_id: String) -> Self {
        Self {
            success: true,
            message_id,
            delivered_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            response: None,
            error: None,
        }
    }

    /// Create a failed result
    pub fn failed(message_id: String, error: String) -> Self {
        Self {
            success: false,
            message_id,
            delivered_at: 0,
            response: None,
            error: Some(error),
        }
    }
}

/// Message Transport Protocol trait
#[async_trait]
pub trait Mtp: Send + Sync {
    /// Get the protocol name (e.g., "http", "iiop")
    fn name(&self) -> &str;

    /// Get supported URL schemes (e.g., ["http", "https"])
    fn schemes(&self) -> Vec<&str>;

    /// Check if this MTP can handle an address
    fn can_handle(&self, address: &str) -> bool {
        for scheme in self.schemes() {
            if address.starts_with(&format!("{}://", scheme)) {
                return true;
            }
        }
        false
    }

    /// Get current status
    fn status(&self) -> MtpStatus;

    /// Activate the MTP (start listening)
    async fn activate(&mut self, config: &MtpConfig) -> Result<(), MtpError>;

    /// Deactivate the MTP (stop listening)
    async fn deactivate(&mut self) -> Result<(), MtpError>;

    /// Send a message to a remote platform
    async fn send(&self, envelope: &MessageEnvelope) -> Result<DeliveryResult, MtpError>;

    /// Receive incoming messages (polling-based)
    async fn receive(&self) -> Result<Option<MessageEnvelope>, MtpError>;

    /// Get MTP statistics
    fn stats(&self) -> MtpStats;
}

/// MTP statistics
#[derive(Debug, Clone, Default)]
pub struct MtpStats {
    /// Messages sent
    pub messages_sent: u64,

    /// Messages received
    pub messages_received: u64,

    /// Failed sends
    pub send_failures: u64,

    /// Failed receives
    pub receive_failures: u64,

    /// Bytes sent
    pub bytes_sent: u64,

    /// Bytes received
    pub bytes_received: u64,

    /// Active connections
    pub active_connections: usize,
}

/// Registry of available MTPs
pub struct MtpRegistry {
    /// Registered MTPs
    mtps: HashMap<String, Arc<RwLock<Box<dyn Mtp>>>>,

    /// Default MTP name
    default_mtp: Option<String>,
}

impl MtpRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            mtps: HashMap::new(),
            default_mtp: None,
        }
    }

    /// Register an MTP
    pub fn register(&mut self, mtp: Box<dyn Mtp>) {
        let name = mtp.name().to_string();
        if self.default_mtp.is_none() {
            self.default_mtp = Some(name.clone());
        }
        self.mtps.insert(name, Arc::new(RwLock::new(mtp)));
    }

    /// Get an MTP by name
    pub fn get(&self, name: &str) -> Option<Arc<RwLock<Box<dyn Mtp>>>> {
        self.mtps.get(name).cloned()
    }

    /// Get the default MTP
    pub fn default_mtp(&self) -> Option<Arc<RwLock<Box<dyn Mtp>>>> {
        self.default_mtp.as_ref().and_then(|name| self.get(name))
    }

    /// Find an MTP that can handle an address
    pub fn find_for_address(&self, address: &str) -> Option<Arc<RwLock<Box<dyn Mtp>>>> {
        // Check the scheme from the address
        let scheme = address.split("://").next().unwrap_or("");
        for (name, mtp) in &self.mtps {
            // Use a heuristic based on name matching scheme
            if name.contains(scheme) || scheme.contains(name) {
                return Some(mtp.clone());
            }
        }
        // Fall back to default
        self.default_mtp()
    }

    /// List registered MTP names
    pub fn list(&self) -> Vec<String> {
        self.mtps.keys().cloned().collect()
    }

    /// Set the default MTP
    pub fn set_default(&mut self, name: &str) -> bool {
        if self.mtps.contains_key(name) {
            self.default_mtp = Some(name.to_string());
            true
        } else {
            false
        }
    }

    /// Activate all MTPs
    pub async fn activate_all(&self, config: &MtpConfig) -> Result<(), MtpError> {
        for mtp in self.mtps.values() {
            let mut mtp = mtp.write().await;
            mtp.activate(config).await?;
        }
        Ok(())
    }

    /// Deactivate all MTPs
    pub async fn deactivate_all(&self) -> Result<(), MtpError> {
        for mtp in self.mtps.values() {
            let mut mtp = mtp.write().await;
            mtp.deactivate().await?;
        }
        Ok(())
    }
}

impl Default for MtpRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtp_config_default() {
        let config = MtpConfig::default();
        assert!(config.enabled);
        assert_eq!(config.connection_timeout_secs, 30);
    }

    #[test]
    fn test_delivery_result_success() {
        let result = DeliveryResult::success("msg-1".to_string());
        assert!(result.success);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_delivery_result_failed() {
        let result = DeliveryResult::failed("msg-1".to_string(), "Connection refused".to_string());
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_mtp_registry() {
        let registry = MtpRegistry::new();
        assert!(registry.list().is_empty());
        assert!(registry.default_mtp().is_none());
    }
}
