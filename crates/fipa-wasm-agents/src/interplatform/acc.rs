// interplatform/acc.rs - Agent Communication Channel
//
//! Agent Communication Channel (ACC)
//!
//! The ACC is responsible for routing messages between agents,
//! whether they are local or on remote platforms.

use super::address::{AddressResolver, AgentAddress, PlatformAddress};
use super::envelope::MessageEnvelope;
use super::mtp::{DeliveryResult, MtpConfig, MtpRegistry};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// ACC errors
#[derive(Debug, Error)]
pub enum AccError {
    #[error("No suitable MTP for address: {0}")]
    NoSuitableMtp(String),

    #[error("Delivery failed: {0}")]
    DeliveryFailed(String),

    #[error("Address resolution failed: {0}")]
    AddressResolutionFailed(String),

    #[error("Message buffering failed: buffer full")]
    BufferFull,

    #[error("Retry limit exceeded")]
    RetryLimitExceeded,

    #[error("Not initialized")]
    NotInitialized,

    #[error("Timeout")]
    Timeout,
}

/// ACC configuration
#[derive(Debug, Clone)]
pub struct AccConfig {
    /// Local platform name
    pub platform_name: String,

    /// Enable message buffering for failed deliveries
    pub enable_buffering: bool,

    /// Maximum buffer size
    pub max_buffer_size: usize,

    /// Maximum retries for failed deliveries
    pub max_retries: u32,

    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,

    /// Delivery timeout in seconds
    pub delivery_timeout_secs: u64,

    /// MTP configuration
    pub mtp_config: MtpConfig,
}

impl Default for AccConfig {
    fn default() -> Self {
        Self {
            platform_name: "fipa-platform".to_string(),
            enable_buffering: true,
            max_buffer_size: 1000,
            max_retries: 3,
            retry_delay_ms: 1000,
            delivery_timeout_secs: 60,
            mtp_config: MtpConfig::default(),
        }
    }
}

/// Buffered message for retry
#[derive(Debug, Clone)]
struct BufferedMessage {
    envelope: MessageEnvelope,
    attempts: u32,
    last_attempt: u64,
    destination: String,
}

/// ACC statistics
#[derive(Debug, Clone, Default)]
pub struct AccStats {
    /// Messages sent locally
    pub local_deliveries: u64,

    /// Messages sent to remote platforms
    pub remote_deliveries: u64,

    /// Failed deliveries
    pub failed_deliveries: u64,

    /// Retried deliveries
    pub retried_deliveries: u64,

    /// Currently buffered messages
    pub buffered_messages: usize,

    /// Messages received from other platforms
    pub received_messages: u64,
}

/// Agent Communication Channel
pub struct Acc {
    /// Configuration
    config: AccConfig,

    /// MTP registry
    mtp_registry: Arc<RwLock<MtpRegistry>>,

    /// Address resolver
    resolver: Arc<RwLock<AddressResolver>>,

    /// Local message delivery callback
    local_delivery: Option<Arc<dyn Fn(MessageEnvelope) + Send + Sync>>,

    /// Buffered messages for retry
    buffer: Arc<RwLock<VecDeque<BufferedMessage>>>,

    /// Statistics
    stats: Arc<AccStatsInner>,

    /// Active flag
    active: Arc<RwLock<bool>>,
}

#[derive(Debug, Default)]
struct AccStatsInner {
    local_deliveries: AtomicU64,
    remote_deliveries: AtomicU64,
    failed_deliveries: AtomicU64,
    retried_deliveries: AtomicU64,
    received_messages: AtomicU64,
}

impl Acc {
    /// Create a new ACC
    pub fn new(config: AccConfig) -> Self {
        let resolver = AddressResolver::new(&config.platform_name);

        Self {
            config,
            mtp_registry: Arc::new(RwLock::new(MtpRegistry::new())),
            resolver: Arc::new(RwLock::new(resolver)),
            local_delivery: None,
            buffer: Arc::new(RwLock::new(VecDeque::new())),
            stats: Arc::new(AccStatsInner::default()),
            active: Arc::new(RwLock::new(false)),
        }
    }

    /// Create with MTP registry
    pub fn with_mtp_registry(mut self, registry: MtpRegistry) -> Self {
        self.mtp_registry = Arc::new(RwLock::new(registry));
        self
    }

    /// Set local delivery callback
    pub fn with_local_delivery<F>(mut self, callback: F) -> Self
    where
        F: Fn(MessageEnvelope) + Send + Sync + 'static,
    {
        self.local_delivery = Some(Arc::new(callback));
        self
    }

    /// Initialize and start the ACC
    pub async fn start(&self) -> Result<(), AccError> {
        let registry = self.mtp_registry.read().await;
        registry
            .activate_all(&self.config.mtp_config)
            .await
            .map_err(|e| AccError::DeliveryFailed(e.to_string()))?;

        let mut active = self.active.write().await;
        *active = true;

        info!("ACC started for platform '{}'", self.config.platform_name);
        Ok(())
    }

    /// Stop the ACC
    pub async fn stop(&self) -> Result<(), AccError> {
        let registry = self.mtp_registry.read().await;
        registry
            .deactivate_all()
            .await
            .map_err(|e| AccError::DeliveryFailed(e.to_string()))?;

        let mut active = self.active.write().await;
        *active = false;

        info!("ACC stopped");
        Ok(())
    }

    /// Send a message
    pub async fn send(&self, envelope: MessageEnvelope) -> Result<DeliveryResult, AccError> {
        let active = self.active.read().await;
        if !*active {
            return Err(AccError::NotInitialized);
        }
        drop(active);

        // Get destination
        let destination = envelope
            .primary_receiver()
            .ok_or_else(|| AccError::DeliveryFailed("No receiver specified".to_string()))?
            .clone();

        // Parse destination address
        let address = AgentAddress::parse(&destination)
            .map_err(|e| AccError::AddressResolutionFailed(e.to_string()))?;

        // Check if local delivery
        let resolver = self.resolver.read().await;
        if resolver.is_local(&address) {
            drop(resolver);
            return self.deliver_local(envelope).await;
        }
        drop(resolver);

        // Remote delivery
        self.deliver_remote(envelope, &destination).await
    }

    /// Deliver locally
    async fn deliver_local(&self, envelope: MessageEnvelope) -> Result<DeliveryResult, AccError> {
        if let Some(ref callback) = self.local_delivery {
            callback(envelope.clone());
            self.stats.local_deliveries.fetch_add(1, Ordering::Relaxed);

            debug!("Local delivery: {}", envelope.id);
            Ok(DeliveryResult::success(envelope.id))
        } else {
            // No local delivery callback - buffer or fail
            warn!("No local delivery callback configured");
            Err(AccError::DeliveryFailed(
                "No local delivery handler".to_string(),
            ))
        }
    }

    /// Deliver to remote platform
    async fn deliver_remote(
        &self,
        envelope: MessageEnvelope,
        destination: &str,
    ) -> Result<DeliveryResult, AccError> {
        // Find suitable MTP
        let registry = self.mtp_registry.read().await;
        let mtp = registry
            .find_for_address(destination)
            .ok_or_else(|| AccError::NoSuitableMtp(destination.to_string()))?;
        drop(registry);

        // Try to send
        let mtp_guard = mtp.read().await;
        match mtp_guard.send(&envelope).await {
            Ok(result) => {
                if result.success {
                    self.stats.remote_deliveries.fetch_add(1, Ordering::Relaxed);
                    debug!("Remote delivery succeeded: {}", envelope.id);
                    Ok(result)
                } else {
                    // Delivery failed - try buffering
                    drop(mtp_guard);
                    self.handle_failed_delivery(envelope, destination, &result.error.unwrap_or_default())
                        .await
                }
            }
            Err(e) => {
                drop(mtp_guard);
                self.handle_failed_delivery(envelope, destination, &e.to_string())
                    .await
            }
        }
    }

    /// Handle failed delivery (buffering/retry)
    async fn handle_failed_delivery(
        &self,
        envelope: MessageEnvelope,
        destination: &str,
        error: &str,
    ) -> Result<DeliveryResult, AccError> {
        warn!("Delivery failed for {}: {}", envelope.id, error);

        if !self.config.enable_buffering {
            self.stats.failed_deliveries.fetch_add(1, Ordering::Relaxed);
            return Ok(DeliveryResult::failed(envelope.id, error.to_string()));
        }

        // Check buffer size
        let mut buffer = self.buffer.write().await;
        if buffer.len() >= self.config.max_buffer_size {
            self.stats.failed_deliveries.fetch_add(1, Ordering::Relaxed);
            return Err(AccError::BufferFull);
        }

        // Add to buffer for retry
        buffer.push_back(BufferedMessage {
            envelope: envelope.clone(),
            attempts: 1,
            last_attempt: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            destination: destination.to_string(),
        });

        debug!("Message {} buffered for retry", envelope.id);
        Ok(DeliveryResult::failed(
            envelope.id,
            format!("Buffered for retry: {}", error),
        ))
    }

    /// Process buffered messages (retry loop)
    pub async fn process_buffer(&self) -> usize {
        let mut processed = 0;
        let mut to_retry = vec![];

        // Get messages ready for retry
        {
            let mut buffer = self.buffer.write().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let retry_threshold = self.config.retry_delay_ms / 1000;

            while let Some(msg) = buffer.pop_front() {
                if now - msg.last_attempt >= retry_threshold {
                    to_retry.push(msg);
                } else {
                    buffer.push_front(msg);
                    break;
                }
            }
        }

        // Retry messages
        for mut msg in to_retry {
            if msg.attempts >= self.config.max_retries {
                self.stats.failed_deliveries.fetch_add(1, Ordering::Relaxed);
                warn!(
                    "Message {} exceeded retry limit, dropping",
                    msg.envelope.id
                );
                continue;
            }

            self.stats.retried_deliveries.fetch_add(1, Ordering::Relaxed);

            match self.deliver_remote(msg.envelope.clone(), &msg.destination).await {
                Ok(result) if result.success => {
                    processed += 1;
                    debug!("Retry succeeded for {}", msg.envelope.id);
                }
                _ => {
                    // Re-buffer
                    msg.attempts += 1;
                    msg.last_attempt = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let mut buffer = self.buffer.write().await;
                    if buffer.len() < self.config.max_buffer_size {
                        buffer.push_back(msg);
                    }
                }
            }
        }

        processed
    }

    /// Receive incoming message from an MTP
    pub async fn receive(&self, envelope: MessageEnvelope) -> Result<(), AccError> {
        self.stats.received_messages.fetch_add(1, Ordering::Relaxed);

        // Check if message is for local agent
        if let Some(receiver) = envelope.primary_receiver() {
            let address = AgentAddress::parse(receiver)
                .map_err(|e| AccError::AddressResolutionFailed(e.to_string()))?;

            let resolver = self.resolver.read().await;
            if resolver.is_local(&address) {
                drop(resolver);
                // Deliver locally
                if let Some(ref callback) = self.local_delivery {
                    callback(envelope);
                    return Ok(());
                }
            }
        }

        // Forward to appropriate destination
        // (In a real implementation, this would handle message forwarding)
        warn!("Message received but no local handler available");
        Ok(())
    }

    /// Register a platform
    pub async fn register_platform(&self, platform: PlatformAddress) {
        let resolver = self.resolver.write().await;
        resolver.register_platform(platform).await;
    }

    /// Get the address resolver
    pub fn resolver(&self) -> Arc<RwLock<AddressResolver>> {
        self.resolver.clone()
    }

    /// Get the MTP registry
    pub fn mtp_registry(&self) -> Arc<RwLock<MtpRegistry>> {
        self.mtp_registry.clone()
    }

    /// Get current statistics
    pub async fn stats(&self) -> AccStats {
        let buffer = self.buffer.read().await.len();

        AccStats {
            local_deliveries: self.stats.local_deliveries.load(Ordering::Relaxed),
            remote_deliveries: self.stats.remote_deliveries.load(Ordering::Relaxed),
            failed_deliveries: self.stats.failed_deliveries.load(Ordering::Relaxed),
            retried_deliveries: self.stats.retried_deliveries.load(Ordering::Relaxed),
            buffered_messages: buffer,
            received_messages: self.stats.received_messages.load(Ordering::Relaxed),
        }
    }

    /// Get buffered message count
    pub async fn buffered_count(&self) -> usize {
        self.buffer.read().await.len()
    }

    /// Clear the buffer
    pub async fn clear_buffer(&self) {
        let mut buffer = self.buffer.write().await;
        buffer.clear();
    }

    /// Get configuration
    pub fn config(&self) -> &AccConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_acc_config_default() {
        let config = AccConfig::default();
        assert!(config.enable_buffering);
        assert_eq!(config.max_retries, 3);
    }

    #[tokio::test]
    async fn test_acc_creation() {
        let config = AccConfig::default();
        let acc = Acc::new(config);

        assert_eq!(acc.buffered_count().await, 0);
    }

    #[tokio::test]
    async fn test_acc_local_delivery() {
        let delivered = Arc::new(AtomicUsize::new(0));
        let delivered_clone = delivered.clone();

        let config = AccConfig::default();
        let acc = Acc::new(config).with_local_delivery(move |_| {
            delivered_clone.fetch_add(1, Ordering::Relaxed);
        });

        acc.start().await.unwrap();

        // Send a local message
        let envelope = MessageEnvelope::new("sender", vec![1, 2, 3]).to("local-agent");

        let result = acc.send(envelope).await.unwrap();
        assert!(result.success);
        assert_eq!(delivered.load(Ordering::Relaxed), 1);

        acc.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_acc_not_initialized() {
        let config = AccConfig::default();
        let acc = Acc::new(config);

        let envelope = MessageEnvelope::new("sender", vec![]).to("receiver");
        let result = acc.send(envelope).await;

        assert!(matches!(result, Err(AccError::NotInitialized)));
    }

    #[tokio::test]
    async fn test_acc_stats() {
        let config = AccConfig::default();
        let acc = Acc::new(config);

        let stats = acc.stats().await;
        assert_eq!(stats.local_deliveries, 0);
        assert_eq!(stats.remote_deliveries, 0);
        assert_eq!(stats.buffered_messages, 0);
    }

    #[tokio::test]
    async fn test_acc_buffer_clear() {
        let config = AccConfig::default();
        let acc = Acc::new(config);

        acc.clear_buffer().await;
        assert_eq!(acc.buffered_count().await, 0);
    }
}
