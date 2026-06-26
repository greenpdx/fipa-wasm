//! Parser errors.

use unl_core::CoreError;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("syntax error at byte {offset}: {message}")]
    Syntax { offset: usize, message: String },
    #[error(transparent)]
    Core(#[from] CoreError),
    /// A concrete syntax that is documented in the manifest but not yet built in
    /// Rev 1 (the UNL/XML document and legacy UNLarium document formats). These
    /// are blocked on access to the surviving corpora so the exact delimiters
    /// can be matched rather than guessed.
    #[error("unsupported format: {0}")]
    Unsupported(&'static str),
}

impl ParseError {
    pub(crate) fn syntax(offset: usize, message: impl Into<String>) -> Self {
        ParseError::Syntax {
            offset,
            message: message.into(),
        }
    }
}
