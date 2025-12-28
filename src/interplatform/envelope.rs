// interplatform/envelope.rs - Message Envelope
//
//! FIPA Message Envelope for inter-platform transport
//!
//! The envelope wraps ACL messages with transport-level metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

/// Transport information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportInfo {
    /// Originating MTP
    pub mtp: String,

    /// Received timestamp
    pub received_at: u64,

    /// Source address (as received)
    pub source_address: String,

    /// Hop count
    pub hops: u32,

    /// Transport-specific metadata
    pub metadata: HashMap<String, String>,
}

impl TransportInfo {
    /// Create new transport info
    pub fn new(mtp: &str, source: &str) -> Self {
        Self {
            mtp: mtp.to_string(),
            received_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            source_address: source.to_string(),
            hops: 0,
            metadata: HashMap::new(),
        }
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Increment hop count
    pub fn increment_hops(&mut self) {
        self.hops += 1;
    }
}

/// Message envelope for inter-platform transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Unique envelope ID
    pub id: String,

    /// Sender agent address
    pub from: String,

    /// Receiver agent addresses
    pub to: Vec<String>,

    /// Intended receiver addresses (may differ from 'to' for forwarding)
    pub intended_receiver: Vec<String>,

    /// ACL representation (how the message is encoded)
    pub acl_representation: String,

    /// Content language
    pub content_language: Option<String>,

    /// Content encoding
    pub content_encoding: Option<String>,

    /// Payload length
    pub payload_length: usize,

    /// Actual message payload (serialized ACL message)
    pub payload: Vec<u8>,

    /// Creation timestamp
    pub created_at: u64,

    /// Transport info (added during transport)
    pub transport_info: Option<TransportInfo>,

    /// Comments
    pub comments: Option<String>,

    /// Custom properties
    pub properties: HashMap<String, String>,
}

impl MessageEnvelope {
    /// Create a new envelope
    pub fn new(from: &str, payload: Vec<u8>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: vec![],
            intended_receiver: vec![],
            acl_representation: "fipa.acl.rep.string.std".to_string(),
            content_language: None,
            content_encoding: None,
            payload_length: payload.len(),
            payload,
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            transport_info: None,
            comments: None,
            properties: HashMap::new(),
        }
    }

    /// Add a receiver
    pub fn to(mut self, receiver: &str) -> Self {
        self.to.push(receiver.to_string());
        self
    }

    /// Add multiple receivers
    pub fn to_many(mut self, receivers: Vec<String>) -> Self {
        self.to.extend(receivers);
        self
    }

    /// Set intended receivers
    pub fn intended_for(mut self, receivers: Vec<String>) -> Self {
        self.intended_receiver = receivers;
        self
    }

    /// Set ACL representation
    pub fn with_acl_representation(mut self, rep: &str) -> Self {
        self.acl_representation = rep.to_string();
        self
    }

    /// Set content language
    pub fn with_content_language(mut self, lang: &str) -> Self {
        self.content_language = Some(lang.to_string());
        self
    }

    /// Set content encoding
    pub fn with_content_encoding(mut self, encoding: &str) -> Self {
        self.content_encoding = Some(encoding.to_string());
        self
    }

    /// Add a comment
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comments = Some(comment.to_string());
        self
    }

    /// Add a property
    pub fn with_property(mut self, key: &str, value: &str) -> Self {
        self.properties.insert(key.to_string(), value.to_string());
        self
    }

    /// Set transport info
    pub fn with_transport_info(mut self, info: TransportInfo) -> Self {
        self.transport_info = Some(info);
        self
    }

    /// Get a property
    pub fn get_property(&self, key: &str) -> Option<&String> {
        self.properties.get(key)
    }

    /// Check if this envelope is for a specific receiver
    pub fn is_for(&self, agent: &str) -> bool {
        self.to.iter().any(|r| r == agent || r.contains(agent))
            || self.intended_receiver.iter().any(|r| r == agent || r.contains(agent))
    }

    /// Get the primary receiver
    pub fn primary_receiver(&self) -> Option<&String> {
        self.to.first().or_else(|| self.intended_receiver.first())
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Serialize to XML (FIPA envelope format)
    pub fn to_xml(&self) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<envelope>\n");
        xml.push_str("  <params index=\"1\">\n");

        // To addresses
        for to in &self.to {
            xml.push_str(&format!("    <to><agent-identifier><name>{}</name></agent-identifier></to>\n", to));
        }

        // From address
        xml.push_str(&format!("    <from><agent-identifier><name>{}</name></agent-identifier></from>\n", self.from));

        // ACL representation
        xml.push_str(&format!("    <acl-representation>{}</acl-representation>\n", self.acl_representation));

        // Content length
        xml.push_str(&format!("    <payload-length>{}</payload-length>\n", self.payload_length));

        // Date
        xml.push_str(&format!("    <date>{}</date>\n", self.created_at));

        // Intended receivers
        for receiver in &self.intended_receiver {
            xml.push_str(&format!("    <intended-receiver><agent-identifier><name>{}</name></agent-identifier></intended-receiver>\n", receiver));
        }

        xml.push_str("  </params>\n");
        xml.push_str("</envelope>\n");

        xml
    }
}

/// Builder for message envelopes
pub struct EnvelopeBuilder {
    from: String,
    to: Vec<String>,
    payload: Vec<u8>,
    acl_representation: String,
    content_language: Option<String>,
    properties: HashMap<String, String>,
}

impl EnvelopeBuilder {
    /// Create a new builder
    pub fn new(from: &str) -> Self {
        Self {
            from: from.to_string(),
            to: vec![],
            payload: vec![],
            acl_representation: "fipa.acl.rep.string.std".to_string(),
            content_language: None,
            properties: HashMap::new(),
        }
    }

    /// Add a receiver
    pub fn to(mut self, receiver: &str) -> Self {
        self.to.push(receiver.to_string());
        self
    }

    /// Set payload
    pub fn payload(mut self, data: Vec<u8>) -> Self {
        self.payload = data;
        self
    }

    /// Set payload from string
    pub fn payload_string(mut self, data: &str) -> Self {
        self.payload = data.as_bytes().to_vec();
        self
    }

    /// Set ACL representation
    pub fn acl_representation(mut self, rep: &str) -> Self {
        self.acl_representation = rep.to_string();
        self
    }

    /// Set content language
    pub fn content_language(mut self, lang: &str) -> Self {
        self.content_language = Some(lang.to_string());
        self
    }

    /// Add a property
    pub fn property(mut self, key: &str, value: &str) -> Self {
        self.properties.insert(key.to_string(), value.to_string());
        self
    }

    /// Build the envelope
    pub fn build(self) -> MessageEnvelope {
        let mut envelope = MessageEnvelope::new(&self.from, self.payload)
            .with_acl_representation(&self.acl_representation);

        for receiver in self.to {
            envelope = envelope.to(&receiver);
        }

        if let Some(lang) = self.content_language {
            envelope = envelope.with_content_language(&lang);
        }

        for (key, value) in self.properties {
            envelope = envelope.with_property(&key, &value);
        }

        envelope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_creation() {
        let envelope = MessageEnvelope::new("sender@platform1", vec![1, 2, 3])
            .to("receiver@platform2")
            .with_content_language("fipa-sl");

        assert_eq!(envelope.from, "sender@platform1");
        assert_eq!(envelope.to, vec!["receiver@platform2"]);
        assert_eq!(envelope.payload, vec![1, 2, 3]);
        assert_eq!(envelope.payload_length, 3);
    }

    #[test]
    fn test_envelope_is_for() {
        let envelope = MessageEnvelope::new("sender", vec![])
            .to("agent1@platform")
            .to("agent2@platform");

        assert!(envelope.is_for("agent1"));
        assert!(envelope.is_for("agent2"));
        assert!(!envelope.is_for("agent3"));
    }

    #[test]
    fn test_envelope_serialization() {
        let envelope = MessageEnvelope::new("sender", vec![1, 2, 3])
            .to("receiver")
            .with_property("key", "value");

        let bytes = envelope.to_bytes().unwrap();
        let restored = MessageEnvelope::from_bytes(&bytes).unwrap();

        assert_eq!(restored.from, "sender");
        assert_eq!(restored.payload, vec![1, 2, 3]);
        assert_eq!(restored.get_property("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_envelope_builder() {
        let envelope = EnvelopeBuilder::new("sender@p1")
            .to("receiver@p2")
            .payload_string("Hello, World!")
            .content_language("fipa-sl")
            .property("priority", "high")
            .build();

        assert_eq!(envelope.from, "sender@p1");
        assert_eq!(envelope.to, vec!["receiver@p2"]);
        assert_eq!(envelope.content_language, Some("fipa-sl".to_string()));
    }

    #[test]
    fn test_transport_info() {
        let info = TransportInfo::new("http", "http://platform1.example.com")
            .with_metadata("connection-id", "12345");

        assert_eq!(info.mtp, "http");
        assert_eq!(info.hops, 0);
        assert!(info.metadata.contains_key("connection-id"));
    }

    #[test]
    fn test_envelope_xml() {
        let envelope = MessageEnvelope::new("sender@p1", vec![])
            .to("receiver@p2");

        let xml = envelope.to_xml();
        assert!(xml.contains("<envelope>"));
        assert!(xml.contains("sender@p1"));
        assert!(xml.contains("receiver@p2"));
    }
}
