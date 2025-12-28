// content/codec.rs - Content Language Codecs
//
//! Codec interface for encoding and decoding content
//!
//! Codecs transform between ContentElement structures and byte representations.
//! This follows the FIPA Content Language specification.

use super::ontology::ContentElement;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

/// Codec errors
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("Encoding failed: {0}")]
    EncodingFailed(String),

    #[error("Decoding failed: {0}")]
    DecodingFailed(String),

    #[error("Invalid syntax at position {position}: {message}")]
    SyntaxError { position: usize, message: String },

    #[error("Unsupported content type: {0}")]
    UnsupportedType(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}

/// Codec trait for encoding/decoding content
pub trait Codec: Send + Sync {
    /// Get the MIME type or language identifier
    fn language(&self) -> &str;

    /// Get the codec name for registration
    fn name(&self) -> &str;

    /// Encode a content element to bytes
    fn encode(&self, content: &ContentElement) -> Result<Vec<u8>, CodecError>;

    /// Decode bytes to a content element
    fn decode(&self, bytes: &[u8]) -> Result<ContentElement, CodecError>;

    /// Check if this codec can handle the given language
    fn supports(&self, language: &str) -> bool {
        self.language() == language || self.name() == language
    }
}

/// Registry of available codecs
#[derive(Default)]
pub struct CodecRegistry {
    codecs: HashMap<String, Arc<dyn Codec>>,
}

impl CodecRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            codecs: HashMap::new(),
        }
    }

    /// Register a codec
    pub fn register(&mut self, codec: Arc<dyn Codec>) {
        let name = codec.name().to_string();
        self.codecs.insert(name, codec);
    }

    /// Get a codec by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Codec>> {
        self.codecs.get(name).cloned()
    }

    /// Find a codec that supports a language
    pub fn find_by_language(&self, language: &str) -> Option<Arc<dyn Codec>> {
        self.codecs
            .values()
            .find(|c| c.supports(language))
            .cloned()
    }

    /// List all registered codec names
    pub fn list(&self) -> Vec<String> {
        self.codecs.keys().cloned().collect()
    }

    /// Check if a codec is registered
    pub fn contains(&self, name: &str) -> bool {
        self.codecs.contains_key(name)
    }

    /// Remove a codec
    pub fn remove(&mut self, name: &str) -> Option<Arc<dyn Codec>> {
        self.codecs.remove(name)
    }
}

/// Helper trait for building encoded content
pub trait ContentEncoder {
    /// Start a new concept
    fn concept(&mut self, name: &str) -> &mut Self;

    /// Add a slot with string value
    fn slot_string(&mut self, name: &str, value: &str) -> &mut Self;

    /// Add a slot with integer value
    fn slot_int(&mut self, name: &str, value: i64) -> &mut Self;

    /// Add a slot with float value
    fn slot_float(&mut self, name: &str, value: f64) -> &mut Self;

    /// Add a slot with boolean value
    fn slot_bool(&mut self, name: &str, value: bool) -> &mut Self;

    /// End the current concept
    fn end(&mut self) -> &mut Self;

    /// Build the final encoded bytes
    fn build(&self) -> Vec<u8>;
}

/// Simple string-based encoder for debugging
pub struct StringEncoder {
    buffer: String,
    indent: usize,
}

impl StringEncoder {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            indent: 0,
        }
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent {
            self.buffer.push_str("  ");
        }
    }
}

impl Default for StringEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentEncoder for StringEncoder {
    fn concept(&mut self, name: &str) -> &mut Self {
        self.write_indent();
        self.buffer.push('(');
        self.buffer.push_str(name);
        self.buffer.push('\n');
        self.indent += 1;
        self
    }

    fn slot_string(&mut self, name: &str, value: &str) -> &mut Self {
        self.write_indent();
        self.buffer.push(':');
        self.buffer.push_str(name);
        self.buffer.push(' ');
        self.buffer.push('"');
        // Escape quotes in value
        for c in value.chars() {
            if c == '"' {
                self.buffer.push('\\');
            }
            self.buffer.push(c);
        }
        self.buffer.push('"');
        self.buffer.push('\n');
        self
    }

    fn slot_int(&mut self, name: &str, value: i64) -> &mut Self {
        self.write_indent();
        self.buffer.push(':');
        self.buffer.push_str(name);
        self.buffer.push(' ');
        self.buffer.push_str(&value.to_string());
        self.buffer.push('\n');
        self
    }

    fn slot_float(&mut self, name: &str, value: f64) -> &mut Self {
        self.write_indent();
        self.buffer.push(':');
        self.buffer.push_str(name);
        self.buffer.push(' ');
        self.buffer.push_str(&value.to_string());
        self.buffer.push('\n');
        self
    }

    fn slot_bool(&mut self, name: &str, value: bool) -> &mut Self {
        self.write_indent();
        self.buffer.push(':');
        self.buffer.push_str(name);
        self.buffer.push(' ');
        self.buffer.push_str(if value { "true" } else { "false" });
        self.buffer.push('\n');
        self
    }

    fn end(&mut self) -> &mut Self {
        if self.indent > 0 {
            self.indent -= 1;
        }
        self.write_indent();
        self.buffer.push(')');
        self.buffer.push('\n');
        self
    }

    fn build(&self) -> Vec<u8> {
        self.buffer.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_registry() {
        let mut registry = CodecRegistry::new();
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_string_encoder() {
        let mut encoder = StringEncoder::new();
        encoder
            .concept("agent-description")
            .slot_string("name", "test-agent")
            .slot_int("priority", 5)
            .slot_bool("active", true)
            .end();

        let result = String::from_utf8(encoder.build()).unwrap();
        assert!(result.contains("agent-description"));
        assert!(result.contains(":name \"test-agent\""));
        assert!(result.contains(":priority 5"));
        assert!(result.contains(":active true"));
    }

    #[test]
    fn test_string_encoder_nested() {
        let mut encoder = StringEncoder::new();
        encoder
            .concept("outer")
            .slot_string("value", "outer-value")
            .end();

        let result = String::from_utf8(encoder.build()).unwrap();
        assert!(result.contains("(outer"));
        assert!(result.contains(")"));
    }
}
