//! # unl-parser
//!
//! Reads and writes the UNL concrete syntaxes, converting between text and the
//! [`unl_core`] data model (`~/SOURCES_MANIFEST.md` §3).
//!
//! ## What Rev 1 supports
//! - **Table format** — one relation per line, UWs inline: `agt(kill, Peter)`.
//! - **List format** — `[W]` node declarations + `[R]` relation block.
//! - **Serialization** of both, via [`serialize`] / [`to_table`] / [`to_list`].
//!
//! The **round-trip invariant** is property-tested: `parse(serialize(g)) == g`
//! for any graph in *canonical form* for the chosen format. Canonical form means
//! the UW fields that Rev 1 does not encode inline (`Uci::Ucn { lang }`, a UW's
//! `node_id`/`scope`) are unset, and references match the format (table = inline
//! UWs, list = id references). See the `grammar` module docs.
//!
//! ## Deviations from the manifest (flagged)
//! - The manifest specifies a `nom`/`quick-xml` implementation; Rev 1 uses a
//!   small hand-written recursive-descent parser instead (one less dependency,
//!   full control over this compact grammar). The public API is unchanged.
//! - `ToUnl` (defined in `unl-core`) cannot be implemented here for `UnlGraph`:
//!   both the trait and the type are foreign to this crate, so the orphan rule
//!   forbids it. Serialization is exposed as free functions instead; `ToUnl`
//!   would be implemented in `unl-core` if serialization ever moves there.
//! - [`parse_legacy_document`] reads the surviving UNLarium `[D]/[S]/{org}/
//!   {unl}` corpus format (see the `legacy` module); the AESOP corpus is the
//!   golden fixture. [`parse_document`] / [`serialize_document`] handle the
//!   UNL/XML document format (spec §6, `quick-xml`), with the graph body in the
//!   legacy inline serialization.

mod error;
mod grammar;
mod legacy;
mod list;
mod table;
mod xml;

#[cfg(test)]
mod tests;

pub use error::ParseError;
pub use legacy::{parse_legacy_document, serialize_legacy_document};
pub use list::{parse_list, serialize_list};
pub use table::{parse_table, serialize_table};
pub use xml::{parse_document, serialize_document};

use unl_core::{UnlFormat, UnlGraph};

/// Parse a single UNL sentence in table or list format. Auto-detects: the
/// presence of a `[W]` block means list format, otherwise table.
pub fn parse_sentence(input: &str) -> Result<UnlGraph, ParseError> {
    if input.contains("[W]") {
        parse_list(input)
    } else {
        parse_table(input)
    }
}

/// Serialize a graph to the chosen concrete syntax.
pub fn serialize(graph: &UnlGraph, format: UnlFormat) -> String {
    match format {
        UnlFormat::Table => serialize_table(graph),
        UnlFormat::List => serialize_list(graph),
    }
}

/// Convenience: serialize to table format.
pub fn to_table(graph: &UnlGraph) -> String {
    serialize_table(graph)
}

/// Convenience: serialize to list format.
pub fn to_list(graph: &UnlGraph) -> String {
    serialize_list(graph)
}

