// interplatform/mod.rs - FIPA Inter-Platform Communication
//
//! Inter-Platform Communication Framework
//!
//! This module provides cross-platform agent communication following FIPA specs:
//!
//! - **MTP (Message Transport Protocol)**: Pluggable transport protocols
//! - **ACC (Agent Communication Channel)**: Routes messages between platforms
//! - **Address Resolution**: Resolves agent addresses across platforms
//!
//! # Architecture
//!
//! ```text
//! +------------------+     +------------------+     +------------------+
//! |  Local Agent     |---->|       ACC        |---->|  Remote Platform |
//! +------------------+     +------------------+     +------------------+
//!                                  |
//!                                  v
//!                          +------------------+
//!                          |  MTP Registry    |
//!                          +------------------+
//!                          | - HTTP MTP       |
//!                          | - gRPC MTP       |
//!                          | - Custom MTPs    |
//!                          +------------------+
//! ```
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::interplatform::{ACC, HttpMtp, MtpRegistry};
//!
//! // Create MTP registry with HTTP support
//! let mut registry = MtpRegistry::new();
//! registry.register(Box::new(HttpMtp::new()));
//!
//! // Create ACC
//! let acc = ACC::new(registry, "local-platform");
//!
//! // Send message to remote platform
//! let envelope = MessageEnvelope::new(message)
//!     .to("agent@remote-platform.example.com:8080");
//! acc.send(envelope).await?;
//! ```

pub mod acc;
pub mod address;
pub mod envelope;
pub mod http_mtp;
pub mod mtp;

pub use acc::{Acc, AccConfig, AccError, AccStats};
pub use address::{AgentAddress, PlatformAddress, AddressResolver, AddressError};
pub use envelope::{MessageEnvelope, EnvelopeBuilder, TransportInfo};
pub use http_mtp::HttpMtp;
pub use mtp::{Mtp, MtpConfig, MtpError, MtpRegistry, MtpStatus};

use thiserror::Error;

/// Inter-platform communication errors
#[derive(Debug, Error)]
pub enum InterplatformError {
    #[error("MTP error: {0}")]
    Mtp(#[from] MtpError),

    #[error("ACC error: {0}")]
    Acc(#[from] AccError),

    #[error("Address error: {0}")]
    Address(#[from] AddressError),

    #[error("No suitable MTP found for address: {0}")]
    NoSuitableMtp(String),

    #[error("Platform unreachable: {0}")]
    PlatformUnreachable(String),

    #[error("Message delivery failed: {0}")]
    DeliveryFailed(String),

    #[error("Timeout")]
    Timeout,
}

/// Inter-platform communication configuration
#[derive(Debug, Clone)]
pub struct InterplatformConfig {
    /// Local platform name
    pub platform_name: String,

    /// Local platform addresses (for incoming connections)
    pub listen_addresses: Vec<String>,

    /// Enable HTTP MTP
    pub enable_http_mtp: bool,

    /// HTTP MTP port
    pub http_port: u16,

    /// Connection timeout in seconds
    pub connection_timeout_secs: u64,

    /// Message timeout in seconds
    pub message_timeout_secs: u64,

    /// Maximum retries for failed deliveries
    pub max_retries: u32,

    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,

    /// Enable message buffering
    pub enable_buffering: bool,

    /// Maximum buffer size
    pub max_buffer_size: usize,
}

impl Default for InterplatformConfig {
    fn default() -> Self {
        Self {
            platform_name: "fipa-platform".to_string(),
            listen_addresses: vec!["0.0.0.0:8080".to_string()],
            enable_http_mtp: true,
            http_port: 8080,
            connection_timeout_secs: 30,
            message_timeout_secs: 60,
            max_retries: 3,
            retry_delay_ms: 1000,
            enable_buffering: true,
            max_buffer_size: 1000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = InterplatformConfig::default();
        assert!(config.enable_http_mtp);
        assert_eq!(config.http_port, 8080);
        assert_eq!(config.max_retries, 3);
    }
}
