//! # unl-core
//!
//! Foundational types for the UNL semantic hypergraph — the data model the rest
//! of the `unl` stack is built on (see `~/SOURCES_MANIFEST.md` §2).
//!
//! A UNL sentence is a [`UnlGraph`]: a set of [`Uw`]s (Universal Words — the
//! nodes) wired together by [`Relation`]s (directed, labelled binary arcs), with
//! each node optionally annotated by an ordered list of [`Attr`]ibutes. A
//! [`UnlDocument`] is an ordered collection of sentences with provenance.
//!
//! This crate has no async and no I/O. Serialization is via `serde`; the
//! canonical *text* form (the UNL spec syntaxes) lives in `unl-parser`.
//!
//! ## Module map
//! - [`uw`] — Universal Words: [`Uci`], [`Uw`], [`LexCategory`]
//! - [`attr`] — universal attributes: [`Attr`], [`AttrList`]
//! - [`relation`] — relation tags + the subsumption hierarchy: [`RelationTag`], [`Relation`]
//! - [`graph`] — [`UnlGraph`], [`UnlDocument`] and the id newtypes
//! - [`ucl`] — UCL id-range classification ([`UclRange`]) for the open-core boundary
//! - [`traits`] — behavioural surface: [`ToUnl`], [`SemanticGraph`], [`UnlEquivalent`]
//! - [`error`] — [`CoreError`]

pub mod attr;
pub mod error;
pub mod graph;
pub mod relation;
pub mod traits;
pub mod ucl;
pub mod uw;

pub use attr::{Attr, AttrList};
pub use error::CoreError;
pub use graph::{DocMetadata, Lang, NodeId, ScopeId, UnlDocument, UnlGraph, UnlSentence};
pub use relation::{NodeRef, Relation, RelationTag};
pub use traits::{SemanticGraph, ToUnl, UnlEquivalent, UnlFormat};
pub use ucl::UclRange;
pub use uw::{LexCategory, Uci, UcnSuffix, Uw};
