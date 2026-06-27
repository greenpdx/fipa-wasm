//! # unl-kb
//!
//! The knowledge-base layer (`~/SOURCES_MANIFEST.md` §4): the [`KnowledgeBase`]
//! trait that resolves concept identities and answers definitional / ontological
//! queries, plus [`MemKb`], an in-memory test double seeded from TOML.
//!
//! The trait is the open interface; the heavy implementations (`WordNetKb`
//! seed, `RocksKb` embedded store) build the *data* behind it. `MemKb` lets the
//! rest of the stack — the validator and the UNLizer — be developed and tested
//! against a deterministic, dependency-light KB before that data exists.
//!
//! The trait is synchronous and object-safe (`&dyn KnowledgeBase`), as the
//! validator consumes it. Remote/async KBs are a separate concern layered on
//! top, not a change to this trait.

mod mem;
mod sled_kb;
mod wordnet;

pub use mem::{ConceptSeed, MemKb};
pub use sled_kb::{BuildStats, SledKb};
pub use wordnet::WordNetKb;

use unl_core::{Lang, LexCategory, Relation, RelationTag, Uci};

/// A UNL knowledge base: resolves concept identities and answers definitional
/// and ontological queries.
pub trait KnowledgeBase {
    /// Resolve a UCN (or UCL) to its canonical UCL, if known.
    /// `cat(icl>feline)` => `ucl 102121620`. Returns `None` for an unknown or
    /// non-resolvable identity (temporary, null).
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError>;

    /// The definitional relations of a concept (its intension), e.g. its `icl`
    /// hypernyms and `iof` instance-of links.
    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError>;

    /// Lexical category and basic features of a concept.
    fn features(&self, concept: &Uci) -> Result<Option<ConceptFeatures>, KbError>;

    /// Is `sub` an `icl`/`iof`-descendant (a kind/instance of) `sup`? Walks the
    /// ontology. Reflexive: a concept is a kind of itself.
    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError>;

    /// Degree of certainty (0..=255) that the given relation holds between two
    /// concepts. Used by the validator to score candidate UNLizations; `0`
    /// means "no evidence / disallowed".
    fn relation_certainty(
        &self,
        tag: RelationTag,
        source: &Uci,
        target: &Uci,
    ) -> Result<u8, KbError>;

    /// Candidate UWs for a natural-language lemma in a given language. Drives
    /// the UNLizer's word-sense disambiguation.
    fn candidates(&self, lemma: &str, lang: Lang) -> Result<Vec<Uci>, KbError>;
}

/// Lexical category plus basic features of a concept.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConceptFeatures {
    pub category: LexCategory,
    /// Whether the concept is abstract (vs. concrete).
    pub abstract_: bool,
    /// Human-readable definition.
    pub gloss: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum KbError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("concept not found: {0:?}")]
    NotFound(Uci),
}
