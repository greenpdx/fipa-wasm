//! Diagnostics emitted by validation (spec §5).

use serde::{Deserialize, Serialize};
use unl_core::NodeId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagCode,
    pub message: String,
    pub location: Option<NodeId>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: DiagCode, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            code,
            message: message.into(),
            location: None,
        }
    }

    /// Attach a node location.
    pub fn at(mut self, location: Option<NodeId>) -> Self {
        self.location = location;
        self
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagCode {
    /// A relation points at a node id that is not declared.
    DanglingReference,
    /// Mutually-exclusive attributes on one node (e.g. `@singular` + `@plural`).
    IncompatibleAttributes,
    /// A relation contradicts the KB ontology (e.g. an `icl` link that `is_a`
    /// disproves).
    RelationTypeViolation,
    /// A `00` pro-form that was never resolved.
    UnsaturatedProForm,
    /// A duplicated relation arc.
    Redundancy,
    /// No `@entry`, and the head cannot be inferred unambiguously.
    AmbiguousEntry,
    /// A UW whose concept does not resolve in the KB.
    UnknownConcept,
    /// A concept, relation, or attribute outside the agent's vocabulary — the
    /// agent has no word for it, so the message is not-understood.
    OutOfVocabulary,
}
