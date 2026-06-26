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
//! - [`parse_document`] (UNL/XML) and [`parse_legacy_document`] (the UNLarium
//!   plain-text export) return [`ParseError::Unsupported`] — they are blocked on
//!   access to the surviving corpora so the exact delimiters are matched, not
//!   guessed.

mod error;
mod grammar;
mod list;
mod table;

#[cfg(test)]
mod tests;

pub use error::ParseError;
pub use list::{parse_list, serialize_list};
pub use table::{parse_table, serialize_table};

use unl_core::{UnlDocument, UnlFormat, UnlGraph};

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

/// Parse a full UNL/XML document (spec §6). **Not yet implemented** — see the
/// crate-level deviations note.
pub fn parse_document(_xml: &str) -> Result<UnlDocument, ParseError> {
    Err(ParseError::Unsupported("UNL/XML document format"))
}

/// Parse the legacy plain-text document format (`[D]...[S]...{org}...{unl}...`)
/// emitted by the UNLarium corpus export. **Not yet implemented** — see the
/// crate-level deviations note.
pub fn parse_legacy_document(_text: &str) -> Result<UnlDocument, ParseError> {
    Err(ParseError::Unsupported("legacy UNLarium document format"))
}
