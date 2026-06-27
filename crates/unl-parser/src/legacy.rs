//! Legacy UNLarium document format — the `[D]/[S]/{org}/{unl}` envelope emitted
//! by the surviving corpus export (e.g. the AESOP corpus). This is the format
//! `parse_legacy_document` reads.
//!
//! ```text
//! [D     dn="aa1" did="..."     ]
//! [S:406274]
//! {org:en}
//! The Hare and the Tortoise
//! {/org}
//! {unl}
//! and(102326432:73.@def,     101670092:92.@def)
//! {/unl}
//! [/S]
//! [/D]
//! ```
//!
//! ## What the legacy UW grammar adds over the canonical table format
//! - **Node-id suffix**: `102326432:73` — a UW carrying a local node id.
//! - **Scope-reference args**: `:01.@def` — references the hyper-node defined by
//!   a scoped relation `and:01(...)`. Modeled as an inline [`Uw`] with `uci =
//!   Null`, `scope = Some(01)`, so its attributes are preserved.
//! - **Null-with-id**: `00:3F.@1`.
//! - **Unquoted multiword headwords**: `take a nap.@past`, `go on`.
//!
//! ## Normalizations applied on ingest (documented, lossy at the text level)
//! - **Legacy relations → 2010 equivalents**: `plt`→`gol`, `plf`→`src`,
//!   `plm`→`via` (the AESOP corpus predates the UNL 2010 place reorganization;
//!   only `plt` occurs in it). The 2010 graph round-trips losslessly.
//! - **Abbreviated attribute labels** not in the canonical set (`@pl`, `@1`,
//!   `@not`, …) become [`Attr::Other`], preserving the label string verbatim.
//! - **2-letter `{org:xx}` codes** are mapped to ISO 639-3 ([`Lang`]).
//! - UWs are kept **inline** (no hoisting into `UnlGraph::nodes` / no node
//!   sharing) in Rev 1; identity unification is future work.

use crate::error::ParseError;
use crate::grammar::{parse_attrs, write_uci, Cursor};
use unl_core::{
    DocMetadata, Lang, NodeId, NodeRef, Relation, RelationTag, ScopeId, Uci, UnlDocument, UnlGraph,
    UnlSentence, Uw,
};

// ---------------------------------------------------------------------------
// Document envelope
// ---------------------------------------------------------------------------

enum Section {
    None,
    Org(Lang, Vec<String>),
    Unl(Vec<String>),
}

/// Parse a legacy UNLarium document into a [`UnlDocument`].
pub fn parse_legacy_document(text: &str) -> Result<UnlDocument, ParseError> {
    let mut doc = UnlDocument::default();
    let mut section = Section::None;
    let mut sentence: Option<UnlSentence> = None;

    for raw in text.lines() {
        let line = raw.trim();
        match &mut section {
            Section::None => {
                if line.starts_with("[D") {
                    doc.metadata = parse_doc_header(line);
                } else if line.starts_with("[/D]") {
                    break;
                } else if let Some(id) = line.strip_prefix("[S:").and_then(|s| s.strip_suffix(']')) {
                    sentence = Some(UnlSentence {
                        id: Some(id.into()),
                        ..Default::default()
                    });
                } else if line.starts_with("[/S]") {
                    if let Some(s) = sentence.take() {
                        doc.sentences.push(s);
                    }
                } else if let Some(code) =
                    line.strip_prefix("{org:").and_then(|s| s.strip_suffix('}'))
                {
                    section = Section::Org(lang_from_code(code)?, Vec::new());
                } else if line.starts_with("{unl}") {
                    section = Section::Unl(Vec::new());
                }
                // stray lines outside any section are ignored
            }
            Section::Org(lang, buf) => {
                if line.starts_with("{/org}") {
                    if let Some(s) = sentence.as_mut() {
                        s.org.push((*lang, buf.join("\n")));
                    }
                    section = Section::None;
                } else {
                    buf.push(line.to_string());
                }
            }
            Section::Unl(buf) => {
                if line.starts_with("{/unl}") {
                    let graph = parse_legacy_graph(&buf.join("\n"))?;
                    if let Some(s) = sentence.as_mut() {
                        s.graph = graph;
                    }
                    section = Section::None;
                } else if !line.is_empty() {
                    buf.push(line.to_string());
                }
            }
        }
    }
    Ok(doc)
}

/// Serialize a [`UnlDocument`] back to the legacy envelope. Round-trips with
/// [`parse_legacy_document`] at the document level.
pub fn serialize_legacy_document(doc: &UnlDocument) -> String {
    let mut out = String::new();
    out.push_str("[D     dn=\"");
    out.push_str(doc.metadata.title.as_deref().unwrap_or(""));
    out.push_str("\" did=\"");
    out.push_str(doc.metadata.date.as_deref().unwrap_or(""));
    out.push_str("\"     ]\n\n");

    for s in &doc.sentences {
        out.push_str("[S:");
        out.push_str(s.id.as_deref().unwrap_or(""));
        out.push_str("]\n");
        for (lang, txt) in &s.org {
            out.push_str("{org:");
            out.push_str(lang.as_str());
            out.push_str("}\n");
            out.push_str(txt);
            out.push_str("\n{/org}\n");
        }
        out.push_str("{unl}\n");
        out.push_str(&serialize_legacy_graph(&s.graph));
        out.push_str("{/unl}\n[/S]\n\n");
    }
    out.push_str("[/D]\n");
    out
}

fn parse_doc_header(line: &str) -> DocMetadata {
    DocMetadata {
        title: attr_value(line, "dn"),
        date: attr_value(line, "did"),
        scheme: Some("UNL".to_string()),
        ..Default::default()
    }
}

/// Extract `key="value"` from a header line.
fn attr_value(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn lang_from_code(code: &str) -> Result<Lang, ParseError> {
    let three = match code {
        c if c.len() == 3 => c,
        "en" => "eng",
        "fr" => "fra",
        "es" => "spa",
        "ru" => "rus",
        "pt" => "por",
        "it" => "ita",
        "de" => "deu",
        "zh" => "zho",
        "ja" => "jpn",
        "ar" => "ara",
        other => {
            return Err(ParseError::syntax(0, format!("unknown language code: {other}")));
        }
    };
    Lang::new(three).map_err(ParseError::from)
}

// ---------------------------------------------------------------------------
// {unl} graph block
// ---------------------------------------------------------------------------

pub(crate) fn parse_legacy_graph(block: &str) -> Result<UnlGraph, ParseError> {
    let mut g = UnlGraph::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        g.add_relation(parse_legacy_relation(line)?);
    }
    Ok(g)
}

pub(crate) fn serialize_legacy_graph(g: &UnlGraph) -> String {
    let mut out = String::new();
    for rel in &g.relations {
        out.push_str(rel.tag.as_str());
        if let Some(s) = &rel.scope {
            out.push(':');
            out.push_str(&s.0);
        }
        out.push('(');
        write_ref(&rel.source, &mut out);
        out.push_str(", ");
        write_ref(&rel.target, &mut out);
        out.push_str(")\n");
    }
    out
}

fn parse_legacy_relation(line: &str) -> Result<Relation, ParseError> {
    let mut c = Cursor::new(line);
    c.skip_spaces();
    let tag_s = c.take_while(|ch| ch.is_ascii_alphabetic());
    if tag_s.is_empty() {
        return Err(c.error("expected a relation tag"));
    }
    let tag = legacy_tag(tag_s).ok_or_else(|| c.error(format!("unknown relation tag: {tag_s}")))?;
    let scope = if c.eat(':') {
        Some(ScopeId(c.take_while(|ch| ch.is_ascii_alphanumeric()).into()))
    } else {
        None
    };
    c.expect('(')?;

    // Args run to the final ')'. UWs contain no commas/parens in this corpus,
    // so split the interior on the top-level comma.
    let rest = c.rest();
    let close = rest.rfind(')').ok_or_else(|| c.error("missing ')'"))?;
    let (a, b) = split_top_comma(&rest[..close])?;
    Ok(Relation {
        tag,
        scope,
        source: NodeRef::Inline(Box::new(parse_legacy_uw(a)?)),
        target: NodeRef::Inline(Box::new(parse_legacy_uw(b)?)),
    })
}

/// Map a tag to a 2010 [`RelationTag`], translating known legacy relations.
fn legacy_tag(s: &str) -> Option<RelationTag> {
    if let Ok(t) = s.parse::<RelationTag>() {
        return Some(t);
    }
    match s {
        "plt" => Some(RelationTag::Gol),
        "plf" => Some(RelationTag::Src),
        "plm" => Some(RelationTag::Via),
        _ => None,
    }
}

fn split_top_comma(s: &str) -> Result<(&str, &str), ParseError> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return Ok((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    Err(ParseError::syntax(0, "expected two relation arguments"))
}

fn parse_legacy_uw(arg: &str) -> Result<Uw, ParseError> {
    let mut c = Cursor::new(arg);
    c.skip_spaces();

    // Scope-reference: ":01.@def".
    if c.eat(':') {
        let scope = c.take_while(|ch| ch.is_ascii_alphanumeric());
        if scope.is_empty() {
            return Err(c.error("expected a scope id after ':'"));
        }
        let (attributes, _entry) = parse_attrs(&mut c)?;
        finish(&c)?;
        return Ok(Uw {
            uci: Uci::Null,
            attributes,
            node_id: None,
            scope: Some(ScopeId(scope.into())),
        });
    }

    let uci = if c.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        let digits = c.take_while(|ch| ch.is_ascii_digit());
        if digits == "00" {
            Uci::Null
        } else {
            Uci::Ucl {
                authority: None,
                id: digits.parse().map_err(|_| c.error("invalid UCL id"))?,
            }
        }
    } else {
        // Multiword headword: read to the node-id ':' or the attribute '.',
        // keeping internal spaces.
        let root = c.take_while(|ch| ch != ':' && ch != '.').trim_end();
        if root.is_empty() {
            return Err(c.error("expected a universal word"));
        }
        Uci::Ucn {
            lang: None,
            root: root.into(),
            suffix: None,
        }
    };

    let node_id = if c.eat(':') {
        Some(NodeId(c.take_while(|ch| ch.is_ascii_alphanumeric()).into()))
    } else {
        None
    };
    let (attributes, _entry) = parse_attrs(&mut c)?;
    finish(&c)?;
    Ok(Uw {
        uci,
        attributes,
        node_id,
        scope: None,
    })
}

fn finish(c: &Cursor) -> Result<(), ParseError> {
    let mut c2 = Cursor::new(c.rest());
    c2.skip_spaces();
    if c2.eof() {
        Ok(())
    } else {
        Err(c.error("trailing characters in universal word"))
    }
}

fn write_ref(r: &NodeRef, out: &mut String) {
    match r {
        NodeRef::Inline(uw) => write_legacy_uw(uw, out),
        NodeRef::Id(n) => out.push_str(&n.0),
        NodeRef::Scope(s) => {
            out.push(':');
            out.push_str(&s.0);
        }
    }
}

fn write_legacy_uw(uw: &Uw, out: &mut String) {
    if let Some(scope) = &uw.scope {
        // Scope reference: the uci is unused.
        out.push(':');
        out.push_str(&scope.0);
    } else {
        write_uci(&uw.uci, out);
        if let Some(nid) = &uw.node_id {
            out.push(':');
            out.push_str(&nid.0);
        }
    }
    for attr in uw.attributes.iter() {
        out.push_str(".@");
        out.push_str(attr.as_label());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use unl_core::{Attr, AttrList};

    fn corpus(lang: &str) -> Option<String> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../data/corpus/aesop/aesop_{lang}.unl"));
        std::fs::read_to_string(p).ok()
    }

    #[test]
    fn parses_first_aesop_sentence() {
        let Some(text) = corpus("en") else {
            eprintln!("skip: AESOP corpus not fetched (cargo run -p xtask -- fetch-aesop)");
            return;
        };
        let doc = parse_legacy_document(&text).unwrap();
        assert_eq!(doc.metadata.title.as_deref(), Some("aa1"));
        assert_eq!(doc.sentences.len(), 13);

        let s0 = &doc.sentences[0];
        assert_eq!(s0.id.as_deref(), Some("406274"));
        assert_eq!(s0.org[0].0, Lang::ENG);
        assert!(s0.org[0].1.contains("Hare"));

        // and(102326432:73.@def, 101670092:92.@def)
        assert_eq!(s0.graph.relations.len(), 1);
        let r = &s0.graph.relations[0];
        assert_eq!(r.tag, RelationTag::And);
        let NodeRef::Inline(uw) = &r.source else {
            panic!("inline UW");
        };
        assert_eq!(uw.uci, Uci::ucl(102326432));
        assert_eq!(uw.node_id, Some(NodeId::from("73")));
        assert!(uw.attributes.contains(&Attr::Def));
    }

    #[test]
    fn graph_roundtrip_over_whole_corpus() {
        let mut checked = 0;
        for lang in ["en", "fr", "es", "ru", "pt", "it"] {
            let Some(text) = corpus(lang) else { continue };
            let doc = parse_legacy_document(&text).unwrap();
            for s in &doc.sentences {
                let unl = serialize_legacy_graph(&s.graph);
                let reparsed = parse_legacy_graph(&unl).unwrap();
                assert_eq!(
                    s.graph, reparsed,
                    "round-trip failed for {lang} {:?}\n{unl}",
                    s.id
                );
                checked += 1;
            }
        }
        if checked > 0 {
            assert!(checked >= 13, "expected at least one full language");
        }
    }

    #[test]
    fn document_roundtrips() {
        let Some(text) = corpus("en") else { return };
        let doc = parse_legacy_document(&text).unwrap();
        let text2 = serialize_legacy_document(&doc);
        let doc2 = parse_legacy_document(&text2).unwrap();
        assert_eq!(doc, doc2);
    }

    #[test]
    fn legacy_plt_maps_to_gol_and_multiword_preserved() {
        let g = parse_legacy_graph("plt(go on.@past,     end.@def)").unwrap();
        assert_eq!(g.relations[0].tag, RelationTag::Gol);
        let NodeRef::Inline(uw) = &g.relations[0].source else {
            panic!()
        };
        assert_eq!(uw.uci, Uci::ucn("go on"));
        assert_eq!(uw.attributes, AttrList(vec![Attr::Past]));
    }

    #[test]
    fn scope_reference_and_null_with_id_roundtrip() {
        let g = parse_legacy_graph("mod(:01.@def,     00:3F.@1)").unwrap();
        let r = &g.relations[0];
        let NodeRef::Inline(src) = &r.source else { panic!() };
        assert_eq!(src.scope, Some(ScopeId::from("01")));
        assert!(src.attributes.contains(&Attr::Def));
        let NodeRef::Inline(tgt) = &r.target else { panic!() };
        assert_eq!(tgt.uci, Uci::Null);
        assert_eq!(tgt.node_id, Some(NodeId::from("3F")));
        assert_eq!(tgt.attributes, AttrList(vec![Attr::Other("1".into())]));

        let s = serialize_legacy_graph(&g);
        assert_eq!(parse_legacy_graph(&s).unwrap(), g);
    }
}
