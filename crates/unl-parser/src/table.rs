//! Table format (spec §5): one relation per line, with UWs written inline.
//!
//! Example: `aoj(300986027, 102121620.@def)`.
//!
//! In this format the graph is carried entirely by the relations' inline UWs;
//! [`UnlGraph::nodes`] is left empty and [`UnlGraph::entry`] is `None` (table
//! canonical form). Lines beginning with `#` and blank lines are ignored.

use crate::error::ParseError;
use crate::grammar::{parse_opt_scope, parse_tag, parse_uw, write_uw, Cursor};
use unl_core::{NodeRef, Relation, UnlGraph, Uw};

pub fn parse_table(input: &str) -> Result<UnlGraph, ParseError> {
    let mut c = Cursor::new(input);
    let mut g = UnlGraph::new();
    loop {
        skip_trivia(&mut c);
        if c.eof() {
            break;
        }
        g.add_relation(parse_relation(&mut c)?);
    }
    Ok(g)
}

fn skip_trivia(c: &mut Cursor) {
    loop {
        let before = c.offset();
        c.skip_spaces();
        while c.eat('\n') || c.eat('\r') {}
        if c.peek() == Some('#') {
            c.take_while(|ch| ch != '\n');
        }
        if c.offset() == before {
            break;
        }
    }
}

fn parse_relation(c: &mut Cursor) -> Result<Relation, ParseError> {
    let tag = parse_tag(c)?;
    let scope = parse_opt_scope(c);
    c.expect('(')?;
    c.skip_spaces();
    let source = parse_inline(c)?;
    c.skip_spaces();
    c.expect(',')?;
    c.skip_spaces();
    let target = parse_inline(c)?;
    c.skip_spaces();
    c.expect(')')?;
    Ok(Relation {
        tag,
        scope,
        source,
        target,
    })
}

fn parse_inline(c: &mut Cursor) -> Result<NodeRef, ParseError> {
    let (uci, attributes, _entry) = parse_uw(c)?;
    Ok(NodeRef::Inline(Box::new(Uw {
        uci,
        attributes,
        node_id: None,
        scope: None,
    })))
}

pub fn serialize_table(g: &UnlGraph) -> String {
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

fn write_ref(r: &NodeRef, out: &mut String) {
    match r {
        NodeRef::Inline(uw) => write_uw(uw, false, out),
        NodeRef::Id(n) => out.push_str(&n.0),
        NodeRef::Scope(s) => {
            out.push(':');
            out.push_str(&s.0);
        }
    }
}
