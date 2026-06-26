//! Errors raised by `unl-core` constructors and validators.

use crate::graph::NodeId;

/// Errors produced when building or checking core types. Higher layers
/// (`unl-parser`, `unl-validator`) wrap this via `#[from]`.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid UCL id: {0}")]
    InvalidUcl(String),
    #[error("malformed UCN: {0}")]
    InvalidUcn(String),
    #[error("unknown relation tag: {0}")]
    UnknownRelation(String),
    #[error("dangling node reference: {0:?}")]
    DanglingRef(NodeId),
    #[error("invalid ISO 639-3 language code: {0}")]
    InvalidLang(String),
}
