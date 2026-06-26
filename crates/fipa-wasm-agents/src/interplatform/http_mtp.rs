// interplatform/http_mtp.rs - HTTP Message Transport Protocol
//
//! HTTP-based Message Transport Protocol
//!
//! Implements FIPA HTTP MTP for cross-platform message delivery.
//! Supports both sending and receiving messages over HTTP/HTTPS.

use super::envelope::MessageEnvelope;
use super::mtp::{DeliveryResult, Mtp, MtpConfig, MtpError, MtpStats, MtpStatus};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// HTTP MTP implementation
pub struct HttpMtp {
    /// Current status
    status: MtpStatus,

    /// HTTP client
    client: reqwest::Client,

    /// Incoming message queue
    incoming: Arc<RwLock<VecDeque<MessageEnvelope>>>,

    /// Statistics
    stats: HttpMtpStats,

    /// Listen address (if server is active)
    listen_address: Option<String>,

    /// Server handle
    server_handle: Option<tokio::task::JoinHandle<()>>,
}

/// HTTP MTP statistics
#[derive(Debug, Default)]
struct HttpMtpStats {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    send_failures: AtomicU64,
    receive_failures: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
}

impl HttpMtp {
    /// Create a new HTTP MTP
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            status: MtpStatus::Inactive,
            client,
            incoming: Arc::new(RwLock::new(VecDeque::new())),
            stats: HttpMtpStats::default(),
            listen_address: None,
            server_handle: None,
        }
    }

    /// Create with custom client configuration
    pub fn with_timeout(timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            status: MtpStatus::Inactive,
            client,
            incoming: Arc::new(RwLock::new(VecDeque::new())),
            stats: HttpMtpStats::default(),
            listen_address: None,
            server_handle: None,
        }
    }

    /// Parse destination URL from agent address
    fn parse_destination(&self, address: &str) -> Result<String, MtpError> {
        // Support various address formats:
        // - http://platform.example.com:8080/acc
        // - agent@http://platform.example.com:8080
        // - http://platform.example.com (defaults to /acc endpoint)

        if address.starts_with("http://") || address.starts_with("https://") {
            let url = if address.contains("/acc") {
                address.to_string()
            } else {
                format!("{}/acc", address.trim_end_matches('/'))
            };
            return Ok(url);
        }

        // Check for agent@url format
        if let Some(pos) = address.find('@') {
            let url_part = &address[pos + 1..];
            if url_part.starts_with("http://") || url_part.starts_with("https://") {
                let url = if url_part.contains("/acc") {
                    url_part.to_string()
                } else {
                    format!("{}/acc", url_part.trim_end_matches('/'))
                };
                return Ok(url);
            }
        }

        Err(MtpError::InvalidAddress(format!(
            "Cannot parse HTTP address from: {}",
            address
        )))
    }

    /// Queue an incoming message
    pub async fn queue_incoming(&self, envelope: MessageEnvelope) {
        let mut queue = self.incoming.write().await;
        queue.push_back(envelope);
        self.stats.messages_received.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for HttpMtp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mtp for HttpMtp {
    fn name(&self) -> &str {
        "http"
    }

    fn schemes(&self) -> Vec<&str> {
        vec!["http", "https"]
    }

    fn status(&self) -> MtpStatus {
        self.status.clone()
    }

    async fn activate(&mut self, config: &MtpConfig) -> Result<(), MtpError> {
        if matches!(self.status, MtpStatus::Active) {
            return Ok(());
        }

        self.status = MtpStatus::Starting;

        // Update client with config timeouts
        self.client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(config.connection_timeout_secs))
            .timeout(Duration::from_secs(config.read_timeout_secs))
            .build()
            .map_err(|e| MtpError::Io(e.to_string()))?;

        // Store listen address
        if let Some(ref addr) = config.listen_address {
            self.listen_address = Some(addr.clone());
            // Note: In a full implementation, we would start an HTTP server here
            // For now, we just mark as active and messages can be queued manually
            info!("HTTP MTP activated on {}", addr);
        }

        self.status = MtpStatus::Active;
        info!("HTTP MTP activated");

        Ok(())
    }

    async fn deactivate(&mut self) -> Result<(), MtpError> {
        self.status = MtpStatus::Stopping;

        // Cancel server if running
        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }

        self.listen_address = None;
        self.status = MtpStatus::Inactive;

        info!("HTTP MTP deactivated");
        Ok(())
    }

    async fn send(&self, envelope: &MessageEnvelope) -> Result<DeliveryResult, MtpError> {
        if !matches!(self.status, MtpStatus::Active) {
            return Err(MtpError::NotActive);
        }

        // Get destination from envelope
        let destination = envelope
            .primary_receiver()
            .ok_or_else(|| MtpError::InvalidAddress("No receiver specified".to_string()))?;

        let url = self.parse_destination(destination)?;

        debug!("Sending message {} to {}", envelope.id, url);

        // Serialize envelope
        let body = envelope
            .to_bytes()
            .map_err(|e| MtpError::Serialization(e.to_string()))?;

        let body_len = body.len() as u64;

        // Send HTTP POST request
        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-FIPA-Envelope-ID", &envelope.id)
            .header("X-FIPA-From", &envelope.from)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                self.stats.send_failures.fetch_add(1, Ordering::Relaxed);
                MtpError::SendFailed(e.to_string())
            })?;

        // Check response
        if response.status().is_success() {
            self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
            self.stats.bytes_sent.fetch_add(body_len, Ordering::Relaxed);

            debug!("Message {} delivered successfully", envelope.id);
            Ok(DeliveryResult::success(envelope.id.clone()))
        } else {
            self.stats.send_failures.fetch_add(1, Ordering::Relaxed);

            let error = format!("HTTP {} - {}", response.status(), response.status().canonical_reason().unwrap_or("Unknown"));
            warn!("Message {} delivery failed: {}", envelope.id, error);

            Ok(DeliveryResult::failed(envelope.id.clone(), error))
        }
    }

    async fn receive(&self) -> Result<Option<MessageEnvelope>, MtpError> {
        let mut queue = self.incoming.write().await;
        Ok(queue.pop_front())
    }

    fn stats(&self) -> MtpStats {
        MtpStats {
            messages_sent: self.stats.messages_sent.load(Ordering::Relaxed),
            messages_received: self.stats.messages_received.load(Ordering::Relaxed),
            send_failures: self.stats.send_failures.load(Ordering::Relaxed),
            receive_failures: self.stats.receive_failures.load(Ordering::Relaxed),
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.stats.bytes_received.load(Ordering::Relaxed),
            active_connections: 0, // HTTP is connectionless
        }
    }
}

/// HTTP MTP server handler (for receiving messages)
pub struct HttpMtpHandler {
    /// Reference to the MTP's incoming queue
    incoming: Arc<RwLock<VecDeque<MessageEnvelope>>>,
}

impl HttpMtpHandler {
    /// Create a new handler
    pub fn new(incoming: Arc<RwLock<VecDeque<MessageEnvelope>>>) -> Self {
        Self { incoming }
    }

    /// Handle an incoming HTTP request
    pub async fn handle_request(&self, body: &[u8]) -> Result<(), MtpError> {
        let envelope: MessageEnvelope =
            serde_json::from_slice(body).map_err(|e| MtpError::ReceiveFailed(e.to_string()))?;

        let mut queue = self.incoming.write().await;
        queue.push_back(envelope);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_mtp_creation() {
        let mtp = HttpMtp::new();
        assert_eq!(mtp.name(), "http");
        assert_eq!(mtp.schemes(), vec!["http", "https"]);
        assert!(matches!(mtp.status(), MtpStatus::Inactive));
    }

    #[test]
    fn test_http_mtp_can_handle() {
        let mtp = HttpMtp::new();
        assert!(mtp.can_handle("http://platform.example.com"));
        assert!(mtp.can_handle("https://platform.example.com:8443"));
        assert!(!mtp.can_handle("iiop://platform.example.com"));
    }

    #[test]
    fn test_parse_destination() {
        let mtp = HttpMtp::new();

        // Direct URL
        let url = mtp.parse_destination("http://platform.example.com:8080").unwrap();
        assert_eq!(url, "http://platform.example.com:8080/acc");

        // URL with /acc already
        let url = mtp.parse_destination("http://platform.example.com/acc").unwrap();
        assert_eq!(url, "http://platform.example.com/acc");

        // Agent@URL format
        let url = mtp.parse_destination("agent1@http://platform.example.com").unwrap();
        assert_eq!(url, "http://platform.example.com/acc");
    }

    #[tokio::test]
    async fn test_http_mtp_activation() {
        let mut mtp = HttpMtp::new();
        let config = MtpConfig::default();

        mtp.activate(&config).await.unwrap();
        assert!(matches!(mtp.status(), MtpStatus::Active));

        mtp.deactivate().await.unwrap();
        assert!(matches!(mtp.status(), MtpStatus::Inactive));
    }

    #[tokio::test]
    async fn test_incoming_queue() {
        let mtp = HttpMtp::new();

        let envelope = MessageEnvelope::new("sender", vec![1, 2, 3]);
        mtp.queue_incoming(envelope.clone()).await;

        let received = mtp.receive().await.unwrap();
        assert!(received.is_some());
        assert_eq!(received.unwrap().payload, vec![1, 2, 3]);

        // Queue should be empty now
        let received = mtp.receive().await.unwrap();
        assert!(received.is_none());
    }

    #[test]
    fn test_stats() {
        let mtp = HttpMtp::new();
        let stats = mtp.stats();

        assert_eq!(stats.messages_sent, 0);
        assert_eq!(stats.messages_received, 0);
    }
}
