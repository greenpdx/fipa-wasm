//! # unl-validator
//!
//! Deterministic, no-ML validation and normalization for UNL graphs (manifest
//! §5). This is what keeps the LLM honest: whatever `unl-llm` produces is
//! checked here for structural and semantic well-formedness before it is
//! trusted. It is the open re-implementation of NORMA and the UNL Verifier.
//!
//! - [`Validate`] runs the [`checks`] (dangling refs, incompatible attributes,
//!   unknown concepts, `icl`/`iof` legality vs the KB, unresolved pro-forms,
//!   redundancy, entry ambiguity) and returns [`Diagnostic`]s.
//! - [`Normalizer`] rewrites a graph to canonical form via a pluggable pipeline
//!   of [`NormRule`]s (Rev 1: voice-collapse, synonym-collapse, proform-resolve).
//! - [`unl_equivalent`] tests meaning-equality by normalizing both graphs and
//!   comparing with the KB-free `UnlEquivalent::unl_eq` (defined in `unl-core`).

pub mod checks;
mod diagnostic;
mod normalize;

pub use diagnostic::{DiagCode, Diagnostic, Severity};
pub use normalize::{
    NormRule, Normalize, Normalizer, ProformResolve, SynonymCollapse, VoiceCollapse,
};

use unl_core::{UnlEquivalent, UnlGraph};
use unl_kb::KnowledgeBase;

/// Structural + semantic validation of a UNL graph against a knowledge base.
pub trait Validate {
    fn validate(&self, kb: &dyn KnowledgeBase) -> Vec<Diagnostic>;
}

impl Validate for UnlGraph {
    fn validate(&self, kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        diags.extend(checks::structural_integrity(self, kb));
        diags.extend(checks::concept_resolution(self, kb));
        diags.extend(checks::relation_legality(self, kb));
        diags.extend(checks::completeness(self, kb));
        diags.extend(checks::non_redundancy(self, kb));
        diags.extend(checks::non_ambiguity(self, kb));
        diags
    }
}

/// True if any diagnostic is an error.
pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

/// Semantic (meaning) equivalence: normalize both graphs with `normalizer`, then
/// compare with the cheap order-independent `unl_eq`.
pub fn unl_equivalent(
    a: &UnlGraph,
    b: &UnlGraph,
    normalizer: &Normalizer,
    kb: &dyn KnowledgeBase,
) -> bool {
    normalizer
        .normalize(a.clone(), kb)
        .unl_eq(&normalizer.normalize(b.clone(), kb))
}

#[cfg(test)]
mod tests;
