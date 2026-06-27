//! UNL/XML document format (spec §6) via `quick-xml`.
//!
//! A self-describing wrapper around the [`UnlDocument`] model. Each sentence
//! carries its original NL (`<org>`), the UNL graph (`<unl>`, in the legacy
//! inline serialization so node ids / scopes survive), and any generated
//! outputs (`<out>`):
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <unl-document>
//!   <metadata title="aa1" scheme="UNL 2010" language="eng"/>
//!   <sentence id="1">
//!     <org lang="eng">The Hare and the Tortoise</org>
//!     <unl>
//! and(102326432:73.@def, 101670092:92.@def)
//!     </unl>
//!   </sentence>
//! </unl-document>
//! ```

use crate::error::ParseError;
use crate::legacy::{parse_legacy_graph, serialize_legacy_graph};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use smol_str::SmolStr;
use unl_core::{DocMetadata, Lang, UnlDocument, UnlSentence};

fn xerr(e: impl std::fmt::Display) -> ParseError {
    ParseError::syntax(0, e.to_string())
}

/// Which text-bearing element is currently open.
enum Field {
    Org(Lang),
    Unl,
    Out(Lang),
}

/// Parse a UNL/XML document into a [`UnlDocument`].
pub fn parse_document(xml: &str) -> Result<UnlDocument, ParseError> {
    let mut reader = Reader::from_str(xml);
    let mut doc = UnlDocument::default();
    let mut sentence: Option<UnlSentence> = None;
    let mut field: Option<Field> = None;
    let mut buf = String::new();

    loop {
        match reader.read_event().map_err(xerr)? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"metadata" => doc.metadata = parse_metadata(&e)?,
                b"sentence" => {
                    sentence = Some(UnlSentence {
                        id: attr(&e, b"id")?,
                        ..Default::default()
                    });
                }
                b"org" => {
                    field = Some(Field::Org(lang_attr(&e)?));
                    buf.clear();
                }
                b"out" => {
                    field = Some(Field::Out(lang_attr(&e)?));
                    buf.clear();
                }
                b"unl" => {
                    field = Some(Field::Unl);
                    buf.clear();
                }
                _ => {}
            },
            Event::Text(e) => {
                if field.is_some() {
                    buf.push_str(&e.unescape().map_err(xerr)?);
                }
            }
            Event::End(e) => {
                match e.name().as_ref() {
                    b"org" => {
                        if let (Some(Field::Org(l)), Some(s)) = (&field, sentence.as_mut()) {
                            s.org.push((*l, buf.trim().to_string()));
                        }
                        field = None;
                    }
                    b"out" => {
                        if let (Some(Field::Out(l)), Some(s)) = (&field, sentence.as_mut()) {
                            s.out.push((*l, buf.trim().to_string()));
                        }
                        field = None;
                    }
                    b"unl" => {
                        if let Some(s) = sentence.as_mut() {
                            s.graph = parse_legacy_graph(buf.trim())?;
                        }
                        field = None;
                    }
                    b"sentence" => {
                        if let Some(s) = sentence.take() {
                            doc.sentences.push(s);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(doc)
}

/// Serialize a [`UnlDocument`] to UNL/XML. Round-trips with [`parse_document`].
pub fn serialize_document(doc: &UnlDocument) -> String {
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<unl-document>\n");

    o.push_str("  <metadata");
    write_opt_attr(&mut o, "title", &doc.metadata.title);
    write_opt_attr(&mut o, "creator", &doc.metadata.creator);
    write_opt_attr(&mut o, "date", &doc.metadata.date);
    if let Some(l) = &doc.metadata.language {
        o.push_str(&format!(" language=\"{}\"", l.as_str()));
    }
    write_opt_attr(&mut o, "scheme", &doc.metadata.scheme);
    write_opt_attr(&mut o, "authority", &doc.metadata.authority);
    o.push_str("/>\n");

    for s in &doc.sentences {
        o.push_str("  <sentence");
        if let Some(id) = &s.id {
            o.push_str(&format!(" id=\"{}\"", escape_attr(id)));
        }
        o.push_str(">\n");
        for (lang, text) in &s.org {
            o.push_str(&format!(
                "    <org lang=\"{}\">{}</org>\n",
                lang.as_str(),
                escape_text(text)
            ));
        }
        o.push_str("    <unl>\n");
        o.push_str(&escape_text(&serialize_legacy_graph(&s.graph)));
        o.push_str("    </unl>\n");
        for (lang, text) in &s.out {
            o.push_str(&format!(
                "    <out lang=\"{}\">{}</out>\n",
                lang.as_str(),
                escape_text(text)
            ));
        }
        o.push_str("  </sentence>\n");
    }
    o.push_str("</unl-document>\n");
    o
}

fn parse_metadata(e: &BytesStart) -> Result<DocMetadata, ParseError> {
    let mut m = DocMetadata::default();
    for a in e.attributes() {
        let a = a.map_err(xerr)?;
        let value = a.unescape_value().map_err(xerr)?.to_string();
        match a.key.as_ref() {
            b"title" => m.title = Some(value),
            b"creator" => m.creator = Some(value),
            b"date" => m.date = Some(value),
            b"language" => m.language = Some(Lang::new(&value).map_err(ParseError::from)?),
            b"scheme" => m.scheme = Some(value),
            b"authority" => m.authority = Some(value),
            _ => {}
        }
    }
    Ok(m)
}

fn attr(e: &BytesStart, key: &[u8]) -> Result<Option<SmolStr>, ParseError> {
    for a in e.attributes() {
        let a = a.map_err(xerr)?;
        if a.key.as_ref() == key {
            return Ok(Some(a.unescape_value().map_err(xerr)?.as_ref().into()));
        }
    }
    Ok(None)
}

fn lang_attr(e: &BytesStart) -> Result<Lang, ParseError> {
    let code = attr(e, b"lang")?.ok_or_else(|| ParseError::syntax(0, "missing lang attribute"))?;
    Lang::new(&code).map_err(ParseError::from)
}

fn write_opt_attr(out: &mut String, key: &str, value: &Option<String>) {
    if let Some(v) = value {
        out.push_str(&format!(" {key}=\"{}\"", escape_attr(v)));
    }
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use unl_core::{NodeRef, Relation, RelationTag, Uci, UnlGraph, Uw};

    /// "Peter killed John" as a legacy inline graph (no [W] declarations).
    fn inline_graph() -> UnlGraph {
        let inline = |uci| NodeRef::Inline(Box::new(Uw::new(uci)));
        let mut g = UnlGraph::new();
        g.add_relation(Relation {
            tag: RelationTag::Agt,
            scope: None,
            source: inline(Uci::ucn("kill")),
            target: inline(Uci::ucn("Peter")),
        });
        g.add_relation(Relation {
            tag: RelationTag::Obj,
            scope: None,
            source: inline(Uci::ucn("kill")),
            target: inline(Uci::ucn("John")),
        });
        g
    }

    fn sample_doc() -> UnlDocument {
        UnlDocument {
            metadata: DocMetadata {
                title: Some("aa1".into()),
                creator: None,
                date: Some("2026-06-27".into()),
                language: Some(Lang::ENG),
                scheme: Some("UNL 2010".into()),
                authority: Some("https://kb.crmep.com".into()),
            },
            sentences: vec![
                UnlSentence {
                    id: Some("1".into()),
                    org: vec![(Lang::ENG, "The Hare and the Tortoise".into())],
                    graph: inline_graph(),
                    out: vec![(Lang::FRA, "Le Li\u{e8}vre et la Tortue".into())],
                },
                UnlSentence {
                    id: Some("2".into()),
                    org: vec![(Lang::ENG, "Peter killed John".into())],
                    graph: inline_graph(),
                    out: vec![],
                },
            ],
        }
    }

    #[test]
    fn xml_document_roundtrips() {
        let doc = sample_doc();
        let xml = serialize_document(&doc);
        assert_eq!(parse_document(&xml).unwrap(), doc);
    }

    #[test]
    fn serialized_xml_has_expected_shape() {
        let xml = serialize_document(&sample_doc());
        assert!(xml.contains("<unl-document>"));
        assert!(xml.contains("language=\"eng\""));
        assert!(xml.contains("<org lang=\"eng\">The Hare and the Tortoise</org>"));
        assert!(xml.contains("agt(kill, Peter)"));
    }

    #[test]
    fn parses_hand_written_xml() {
        let xml = r#"<?xml version="1.0"?>
            <unl-document>
              <metadata scheme="UNL 2010"/>
              <sentence id="42">
                <org lang="eng">cat</org>
                <unl>
icl(cat, animal)
                </unl>
              </sentence>
            </unl-document>"#;
        let doc = parse_document(xml).unwrap();
        assert_eq!(doc.sentences.len(), 1);
        assert_eq!(doc.sentences[0].id.as_deref(), Some("42"));
        assert_eq!(doc.sentences[0].graph.relations[0].tag, RelationTag::Icl);
    }
}
