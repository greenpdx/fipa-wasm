// content/mod.rs - FIPA Content Language Framework
//
//! Content Language and Ontology Framework
//!
//! This module provides a framework for encoding, decoding, and validating
//! message content according to FIPA specifications.
//!
//! # Architecture
//!
//! ```text
//! +------------------+     +------------------+     +------------------+
//! | ContentManager   |---->|     Codec        |---->|    Ontology      |
//! | (encode/decode)  |     | (SL, JSON, etc)  |     | (validation)     |
//! +------------------+     +------------------+     +------------------+
//!         |                        |                        |
//!         v                        v                        v
//! +------------------+     +------------------+     +------------------+
//! | ContentElement   |     |  Encoded Bytes   |     |     Schema       |
//! +------------------+     +------------------+     +------------------+
//! ```
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::content::{ContentManager, ContentElement, Concept};
//!
//! let manager = ContentManager::new()
//!     .with_default_codecs()
//!     .with_default_ontologies();
//!
//! // Create a content element
//! let content = ContentElement::action(
//!     "register",
//!     vec![("agent-name", "my-agent"), ("service", "calculator")],
//! );
//!
//! // Encode to SL format
//! let encoded = manager.encode("fipa-sl", &content)?;
//!
//! // Decode back
//! let decoded = manager.decode("fipa-sl", &encoded)?;
//! ```

pub mod codec;
pub mod ontology;
pub mod sl_codec;

pub use codec::{Codec, CodecError, CodecRegistry};
pub use ontology::{
    Action, Concept, ContentElement, Ontology, OntologyError, OntologyRegistry, Predicate, Schema,
    SchemaField, SchemaType, Term,
};
pub use sl_codec::SlCodec;

use std::sync::Arc;
use thiserror::Error;

/// Content management errors
#[derive(Debug, Error)]
pub enum ContentError {
    #[error("Codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("Ontology error: {0}")]
    Ontology(#[from] OntologyError),

    #[error("Unknown codec: {0}")]
    UnknownCodec(String),

    #[error("Unknown ontology: {0}")]
    UnknownOntology(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),
}

/// Content manager - main entry point for content operations
#[derive(Default)]
pub struct ContentManager {
    /// Registered codecs
    codecs: CodecRegistry,

    /// Registered ontologies
    ontologies: OntologyRegistry,

    /// Default codec name
    default_codec: Option<String>,

    /// Default ontology name
    default_ontology: Option<String>,
}

impl ContentManager {
    /// Create a new content manager
    pub fn new() -> Self {
        Self {
            codecs: CodecRegistry::new(),
            ontologies: OntologyRegistry::new(),
            default_codec: None,
            default_ontology: None,
        }
    }

    /// Add default codecs (SL, JSON)
    pub fn with_default_codecs(mut self) -> Self {
        // Add SL codec
        self.codecs.register(Arc::new(SlCodec::new()));

        // Add JSON codec
        self.codecs.register(Arc::new(JsonCodec::new()));

        self.default_codec = Some("fipa-sl".to_string());
        self
    }

    /// Add default ontologies
    pub fn with_default_ontologies(mut self) -> Self {
        // FIPA Agent Management ontology
        self.ontologies
            .register(Arc::new(ontology::fipa_agent_management()));

        // FIPA Ping ontology
        self.ontologies.register(Arc::new(ontology::fipa_ping()));

        self.default_ontology = Some("fipa-agent-management".to_string());
        self
    }

    /// Register a custom codec
    pub fn register_codec(&mut self, codec: Arc<dyn Codec>) {
        self.codecs.register(codec);
    }

    /// Register a custom ontology
    pub fn register_ontology(&mut self, ontology: Arc<dyn Ontology>) {
        self.ontologies.register(ontology);
    }

    /// Set the default codec
    pub fn set_default_codec(&mut self, name: &str) {
        self.default_codec = Some(name.to_string());
    }

    /// Set the default ontology
    pub fn set_default_ontology(&mut self, name: &str) {
        self.default_ontology = Some(name.to_string());
    }

    /// Encode content using a specific codec
    pub fn encode(&self, codec_name: &str, content: &ContentElement) -> Result<Vec<u8>, ContentError> {
        let codec = self
            .codecs
            .get(codec_name)
            .ok_or_else(|| ContentError::UnknownCodec(codec_name.to_string()))?;

        Ok(codec.encode(content)?)
    }

    /// Encode content using the default codec
    pub fn encode_default(&self, content: &ContentElement) -> Result<Vec<u8>, ContentError> {
        let codec_name = self
            .default_codec
            .as_ref()
            .ok_or_else(|| ContentError::UnknownCodec("no default codec".to_string()))?;

        self.encode(codec_name, content)
    }

    /// Decode content using a specific codec
    pub fn decode(&self, codec_name: &str, bytes: &[u8]) -> Result<ContentElement, ContentError> {
        let codec = self
            .codecs
            .get(codec_name)
            .ok_or_else(|| ContentError::UnknownCodec(codec_name.to_string()))?;

        Ok(codec.decode(bytes)?)
    }

    /// Decode content using the default codec
    pub fn decode_default(&self, bytes: &[u8]) -> Result<ContentElement, ContentError> {
        let codec_name = self
            .default_codec
            .as_ref()
            .ok_or_else(|| ContentError::UnknownCodec("no default codec".to_string()))?;

        self.decode(codec_name, bytes)
    }

    /// Validate content against an ontology
    pub fn validate(
        &self,
        ontology_name: &str,
        content: &ContentElement,
    ) -> Result<(), ContentError> {
        let ontology = self
            .ontologies
            .get(ontology_name)
            .ok_or_else(|| ContentError::UnknownOntology(ontology_name.to_string()))?;

        Ok(ontology.validate(content)?)
    }

    /// Validate content against the default ontology
    pub fn validate_default(&self, content: &ContentElement) -> Result<(), ContentError> {
        let ontology_name = self
            .default_ontology
            .as_ref()
            .ok_or_else(|| ContentError::UnknownOntology("no default ontology".to_string()))?;

        self.validate(ontology_name, content)
    }

    /// Encode and validate content
    pub fn encode_validated(
        &self,
        codec_name: &str,
        ontology_name: &str,
        content: &ContentElement,
    ) -> Result<Vec<u8>, ContentError> {
        // Validate first
        self.validate(ontology_name, content)?;

        // Then encode
        self.encode(codec_name, content)
    }

    /// Get available codec names
    pub fn available_codecs(&self) -> Vec<String> {
        self.codecs.list()
    }

    /// Get available ontology names
    pub fn available_ontologies(&self) -> Vec<String> {
        self.ontologies.list()
    }

    /// Get a schema from an ontology
    pub fn get_schema(&self, ontology_name: &str, schema_name: &str) -> Option<Schema> {
        let ontology = self.ontologies.get(ontology_name)?;
        ontology.get_schema(schema_name).cloned()
    }
}

/// Simple JSON codec for content
pub struct JsonCodec;

impl JsonCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for JsonCodec {
    fn language(&self) -> &str {
        "application/json"
    }

    fn name(&self) -> &str {
        "json"
    }

    fn encode(&self, content: &ContentElement) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(content).map_err(|e| CodecError::EncodingFailed(e.to_string()))
    }

    fn decode(&self, bytes: &[u8]) -> Result<ContentElement, CodecError> {
        serde_json::from_slice(bytes).map_err(|e| CodecError::DecodingFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_manager_creation() {
        let manager = ContentManager::new()
            .with_default_codecs()
            .with_default_ontologies();

        assert!(manager.available_codecs().contains(&"fipa-sl".to_string()));
        assert!(manager.available_codecs().contains(&"json".to_string()));
        assert!(manager
            .available_ontologies()
            .contains(&"fipa-agent-management".to_string()));
    }

    #[test]
    fn test_json_encode_decode() {
        let manager = ContentManager::new().with_default_codecs();

        let content = ContentElement::concept(Concept::new("test").with_slot("key", Term::string("value")));

        let encoded = manager.encode("json", &content).unwrap();
        let decoded = manager.decode("json", &encoded).unwrap();

        assert!(matches!(decoded, ContentElement::Concept(_)));
    }

    #[test]
    fn test_sl_encode_decode() {
        let manager = ContentManager::new().with_default_codecs();

        let content = ContentElement::concept(Concept::new("agent-description").with_slot("name", Term::string("my-agent")));

        let encoded = manager.encode("fipa-sl", &content).unwrap();
        let decoded = manager.decode("fipa-sl", &encoded).unwrap();

        assert!(matches!(decoded, ContentElement::Concept(_)));
    }

    #[test]
    fn test_unknown_codec() {
        let manager = ContentManager::new();
        let content = ContentElement::concept(Concept::new("test"));

        let result = manager.encode("unknown", &content);
        assert!(matches!(result, Err(ContentError::UnknownCodec(_))));
    }
}
