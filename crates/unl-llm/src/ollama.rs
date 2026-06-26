//! [`OllamaBackend`] — the default [`ReasoningBackend`], talking to a local
//! Ollama server (`http://localhost:11434`) over its `/api/chat` endpoint.
//!
//! Ollama runs the model on-device (CPU or GPU), so the whole UNLization
//! pipeline can run with no cloud dependency — the manifest's edge-deployable
//! goal. Pick any installed model by name; structured output is requested via
//! Ollama's `format` field (a JSON schema), which constrains decoding so the
//! relation/attribute enums are honoured.

use crate::{LlmError, Prompt, ReasoningBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Connection + model configuration for a local Ollama server.
pub struct OllamaBackend {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaBackend {
    /// Use the given model name against the default local server
    /// (`http://localhost:11434`).
    pub fn new(model: impl Into<String>) -> Self {
        OllamaBackend {
            client: reqwest::Client::new(),
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
        }
    }

    /// Override the server URL (e.g. a remote or containerised Ollama).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'a serde_json::Value>,
    options: ChatOptions,
}

#[derive(Serialize)]
struct ChatOptions {
    /// Deterministic decoding for a formal pipeline.
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[async_trait]
impl ReasoningBackend for OllamaBackend {
    async fn complete(&self, prompt: &Prompt) -> Result<String, LlmError> {
        let body = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage { role: "system", content: &prompt.system },
                ChatMessage { role: "user", content: &prompt.user },
            ],
            stream: false,
            format: prompt.format.as_ref(),
            options: ChatOptions { temperature: 0.0 },
        };

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| LlmError::Backend(e.to_string()))?;

        let parsed: ChatResponse = resp.json().await?;
        Ok(parsed.message.content)
    }
}
