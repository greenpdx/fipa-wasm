//! # unl-llm
//!
//! LLM-assisted UNLization (`~/SOURCES_MANIFEST.md` §6): natural language in, a
//! **validated** [`UnlGraph`] out, or a structured failure — it never emits an
//! unvalidated graph. The LLM *proposes*; `unl-validator` *disposes*.
//!
//! The reasoning model is held behind the [`ReasoningBackend`] trait, so the
//! formally-constrained envelope is independent of the model. The default
//! backend is [`OllamaBackend`] — a local model over Ollama's HTTP API, in
//! keeping with the manifest's edge-deployable, CPU-only, no-mandatory-cloud
//! design (principle 3 — "LLM-assisted, not LLM-dependent"). Any other endpoint
//! (a hosted API, an Axum proxy) is a second `impl ReasoningBackend`.
//!
//! ## Pipeline ([`LlmUnlizer`])
//! 1. **Ground** — `kb.candidates()` for each content word narrows the UW space
//!    *before* the model runs.
//! 2. **Constrained prompt** — the model is asked for UNL in a JSON schema whose
//!    relation/attribute fields are `enum`s over the *closed sets*; it selects a
//!    tag, it cannot fabricate one.
//! 3. **Decode** → [`UnlGraph`].
//! 4. **Validate** against the KB (`unl-validator`).
//! 5. **Repair** — feed diagnostics back for up to N bounded retries; surviving
//!    structural errors are returned as residuals, not swallowed.

mod error;
mod ollama;
mod unlizer;

#[cfg(test)]
mod tests;

pub use error::LlmError;
pub use ollama::OllamaBackend;
pub use unlizer::LlmUnlizer;

use async_trait::async_trait;
use unl_core::{Lang, UnlGraph};
use unl_validator::Diagnostic;

/// A constrained completion request: a system frame, the user task, and an
/// optional JSON schema the backend must make the output conform to.
#[derive(Clone, Debug)]
pub struct Prompt {
    pub system: String,
    pub user: String,
    /// JSON Schema for structured output (Ollama `format`). `None` => free text.
    pub format: Option<serde_json::Value>,
}

/// The reasoning backend abstraction — keeps `unl-llm` independent of the model.
#[async_trait]
pub trait ReasoningBackend: Send + Sync {
    /// Run one completion, returning the model's raw text (expected to be the
    /// JSON document described by `prompt.format`, when set).
    async fn complete(&self, prompt: &Prompt) -> Result<String, LlmError>;
}

/// The result of UNLization: a validated graph, the diagnostics that survived
/// repair (empty => clean), and an aggregate confidence in [0, 1].
#[derive(Clone, Debug)]
pub struct Unlization {
    pub graph: UnlGraph,
    pub residual_diagnostics: Vec<Diagnostic>,
    pub confidence: f32,
}

/// Converts natural language into UNL graphs, then validates and (optionally)
/// repairs the result.
#[async_trait]
pub trait Unlizer {
    async fn unlize(&self, text: &str, lang: Lang) -> Result<Unlization, LlmError>;
}
