//! Errors from the UNLization pipeline.

use unl_core::CoreError;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// Transport / backend failure (connection, non-2xx, timeout).
    #[error("reasoning backend error: {0}")]
    Backend(String),
    /// The model's output could not be decoded into a graph (bad JSON, unknown
    /// relation tag, dangling structure) and repair attempts were exhausted.
    #[error("could not decode a valid graph from the model: {0}")]
    Decode(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Backend(e.to_string())
    }
}
