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
use unl_kb::{KnowledgeBase, Vocabulary};

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

/// Verify a graph against an agent's [`Vocabulary`]: structurally sound AND every
/// concept, relation, and attribute within the agent's vocabulary. Returns the
/// blocking error diagnostics on failure — an out-of-vocabulary term (or a
/// structural error) makes the message *not-understood*. This is the edge/agent
/// verification path: it needs only the agent's compact vocabulary, not the
/// central KB.
pub fn verify_vocabulary(graph: &UnlGraph, vocab: &Vocabulary) -> Result<(), Vec<Diagnostic>> {
    let mut diags = checks::structural_integrity(graph, vocab);
    diags.extend(checks::vocabulary(graph, vocab));
    diags.retain(|d| d.severity == Severity::Error);
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
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
