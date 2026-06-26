//! List format (spec §5): nodes declared in a `[W]` block, relations in a `[R]`
//! block referencing those node ids.
//!
//! ```text
//! [W]
//! 01: kill.@past.@entry
//! 02: Peter
//! 03: John
//! [/W]
//! [R]
//! agt(01, 02)
//! obj(01, 03)
//! [/R]
//! ```
//!
//! Node identity is the `[W]` declaration id; the `@entry` marker on a
//! declaration sets [`UnlGraph::entry`]. Relations reference ids only
//! (`NodeRef::Id`). Byte offsets in errors are relative to the offending line.

use crate::error::ParseError;
use crate::grammar::{parse_opt_scope, parse_tag, parse_uw, write_uw, Cursor};
use unl_core::{NodeId, NodeRef, Relation, UnlGraph, Uw};

fn is_ident(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

pub fn parse_list(input: &str) -> Result<UnlGraph, ParseError> {
    let w = section(input, "[W]", "[/W]")?;
    let r = section(input, "[R]", "[/R]")?;
    let mut g = UnlGraph::new();

    for line in w.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        parse_w_line(line, &mut g)?;
    }
    for line in r.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        g.add_relation(parse_r_line(line)?);
    }
    Ok(g)
}

/// Return the content between `open` and the next `close`.
fn section<'a>(input: &'a str, open: &str, close: &str) -> Result<&'a str, ParseError> {
    let o = input
        .find(open)
        .ok_or_else(|| ParseError::syntax(0, format!("missing {open} block")))?;
    let start = o + open.len();
    let rel = input[start..]
        .find(close)
        .ok_or_else(|| ParseError::syntax(start, format!("missing {close} for {open} block")))?;
    Ok(&input[start..start + rel])
}

fn parse_w_line(line: &str, g: &mut UnlGraph) -> Result<(), ParseError> {
    let mut c = Cursor::new(line);
    let id = c.take_while(is_ident);
    if id.is_empty() {
        return Err(c.error("expected a node id"));
    }
    c.skip_spaces();
    c.expect(':')?;
    c.skip_spaces();
    let (uci, attributes, entry) = parse_uw(&mut c)?;
    c.skip_spaces();
    if !c.eof() {
        return Err(c.error("trailing characters after UW declaration"));
    }
    let nid = NodeId(id.into());
    g.nodes.insert(
        nid.clone(),
        Uw {
            uci,
            attributes,
            node_id: None,
            scope: None,
        },
    );
    if entry {
        g.entry = Some(nid);
    }
    Ok(())
}

fn parse_r_line(line: &str) -> Result<Relation, ParseError> {
    let mut c = Cursor::new(line);
    let tag = parse_tag(&mut c)?;
    let scope = parse_opt_scope(&mut c);
    c.expect('(')?;
    c.skip_spaces();
    let s1 = c.take_while(is_ident);
    if s1.is_empty() {
        return Err(c.error("expected a source node id"));
    }
    c.skip_spaces();
    c.expect(',')?;
    c.skip_spaces();
    let s2 = c.take_while(is_ident);
    if s2.is_empty() {
        return Err(c.error("expected a target node id"));
    }
    c.skip_spaces();
    c.expect(')')?;
    Ok(Relation {
        tag,
        scope,
        source: NodeRef::Id(NodeId(s1.into())),
        target: NodeRef::Id(NodeId(s2.into())),
    })
}

pub fn serialize_list(g: &UnlGraph) -> String {
    let mut out = String::new();
    out.push_str("[W]\n");
    for (id, uw) in &g.nodes {
        out.push_str(&id.0);
        out.push_str(": ");
        let is_entry = g.entry.as_ref() == Some(id);
        write_uw(uw, is_entry, &mut out);
        out.push('\n');
    }
    out.push_str("[/W]\n");
    out.push_str("[R]\n");
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
    out.push_str("[/R]\n");
    out
}

fn write_ref(r: &NodeRef, out: &mut String) {
    match r {
        NodeRef::Id(n) => out.push_str(&n.0),
        NodeRef::Scope(s) => {
            out.push(':');
            out.push_str(&s.0);
        }
        // Canonical list form uses id references; an inline UW here is degraded
        // input, but render it rather than panic.
        NodeRef::Inline(uw) => write_uw(uw, false, out),
    }
}
