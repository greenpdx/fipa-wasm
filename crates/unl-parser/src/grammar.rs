//! Shared lexical machinery: a byte-offset cursor, plus parse/serialize for a
//! single Universal Word and the attribute suffix. Used by both the table and
//! list format modules.
//!
//! ## Canonical text form
//!
//! A UW renders as `<uci><attrs>` where:
//! - `<uci>` is one of: `00` (null), `"text"` (temporary, with `\` and `"`
//!   escaped), a bare integer (UCL, authority elided), `ucl://auth/id` (UCL with
//!   authority), or `root` / `root(rel>word)` (UCN).
//! - `<attrs>` is zero or more `.@label` segments.
//!
//! The `@entry` marker is handled by the caller (it sets [`UnlGraph::entry`],
//! it is not an [`Attr`]). UCN `lang` and a UW's `node_id`/`scope` fields are not
//! encoded inline in Rev 1 — the round-trip guarantee is stated over graphs in
//! this canonical form (see the crate docs).

use crate::error::ParseError;
use unl_core::{Attr, AttrList, RelationTag, ScopeId, Uci, UcnSuffix};

/// A cursor over the input that tracks the absolute byte offset, so syntax
/// errors can report where they occurred.
pub(crate) struct Cursor<'a> {
    full: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(full: &'a str) -> Self {
        Cursor { full, pos: 0 }
    }

    pub(crate) fn rest(&self) -> &'a str {
        &self.full[self.pos..]
    }

    pub(crate) fn offset(&self) -> usize {
        self.pos
    }

    pub(crate) fn eof(&self) -> bool {
        self.pos >= self.full.len()
    }

    pub(crate) fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub(crate) fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Advance past a known prefix. Caller must have checked [`starts_with`].
    fn advance_str(&mut self, s: &str) {
        debug_assert!(self.starts_with(s));
        self.pos += s.len();
    }

    pub(crate) fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, c: char) -> Result<(), ParseError> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(ParseError::syntax(
                self.pos,
                format!("expected '{c}', found {}", self.found()),
            ))
        }
    }

    pub(crate) fn take_while(&mut self, pred: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if pred(c) {
                self.bump();
            } else {
                break;
            }
        }
        &self.full[start..self.pos]
    }

    /// Skip spaces and tabs (not newlines).
    pub(crate) fn skip_spaces(&mut self) {
        self.take_while(|c| c == ' ' || c == '\t');
    }

    fn found(&self) -> String {
        match self.peek() {
            Some(c) => format!("'{c}'"),
            None => "end of input".to_string(),
        }
    }

    pub(crate) fn error(&self, msg: impl Into<String>) -> ParseError {
        ParseError::syntax(self.pos, msg)
    }
}

const fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

const fn is_ucn_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// Parse a relation tag (lowercase letters) at the cursor.
pub(crate) fn parse_tag(c: &mut Cursor) -> Result<RelationTag, ParseError> {
    let s = c.take_while(|c| c.is_ascii_alphabetic());
    if s.is_empty() {
        return Err(c.error("expected a relation tag"));
    }
    s.parse::<RelationTag>().map_err(ParseError::from)
}

/// Parse an optional `:scope` suffix on a relation tag.
pub(crate) fn parse_opt_scope(c: &mut Cursor) -> Option<ScopeId> {
    if c.eat(':') {
        let s = c.take_while(is_ident);
        Some(ScopeId(s.into()))
    } else {
        None
    }
}

/// Parse a UCI (the identity part of a UW), without attributes.
fn parse_uci(c: &mut Cursor) -> Result<Uci, ParseError> {
    match c.peek() {
        Some('"') => {
            c.eat('"');
            Ok(Uci::Temporary(parse_quoted(c)?.into()))
        }
        _ if c.starts_with("ucl://") => {
            c.advance_str("ucl://");
            let authority = c.take_while(|ch| ch != '/');
            c.expect('/')?;
            let id = parse_u64(c)?;
            Ok(Uci::Ucl {
                authority: Some(authority.into()),
                id,
            })
        }
        Some(ch) if ch.is_ascii_digit() => {
            let digits = c.take_while(|ch| ch.is_ascii_digit());
            // "00" is the reserved null / pro-UW; every real UCL id renders
            // without a leading-zero pad, so this is unambiguous.
            if digits == "00" {
                Ok(Uci::Null)
            } else {
                let id = digits
                    .parse::<u64>()
                    .map_err(|_| c.error(format!("invalid UCL id: {digits}")))?;
                Ok(Uci::Ucl {
                    authority: None,
                    id,
                })
            }
        }
        Some(ch) if is_ucn_start(ch) => {
            let root = c.take_while(is_ident);
            let suffix = if c.peek() == Some('(') {
                c.eat('(');
                let relation = parse_tag(c)?;
                c.expect('>')?;
                let word = c.take_while(|ch| ch != ')');
                c.expect(')')?;
                Some(UcnSuffix {
                    relation,
                    word: word.into(),
                })
            } else {
                None
            };
            Ok(Uci::Ucn {
                lang: None,
                root: root.into(),
                suffix,
            })
        }
        _ => Err(c.error("expected a universal word")),
    }
}

fn parse_u64(c: &mut Cursor) -> Result<u64, ParseError> {
    let digits = c.take_while(|ch| ch.is_ascii_digit());
    if digits.is_empty() {
        return Err(c.error("expected a number"));
    }
    digits
        .parse::<u64>()
        .map_err(|_| c.error(format!("number out of range: {digits}")))
}

/// Parse the body of a quoted temporary, after the opening quote has been eaten.
/// Consumes through the closing quote. Handles `\"` and `\\`.
fn parse_quoted(c: &mut Cursor) -> Result<String, ParseError> {
    let mut out = String::new();
    loop {
        match c.peek() {
            None => return Err(c.error("unterminated quoted temporary")),
            Some('"') => {
                c.eat('"');
                return Ok(out);
            }
            Some('\\') => {
                c.eat('\\');
                match c.peek() {
                    Some(esc) => {
                        out.push(esc);
                        c.bump_one();
                    }
                    None => return Err(c.error("dangling escape in quoted temporary")),
                }
            }
            Some(ch) => {
                out.push(ch);
                c.bump_one();
            }
        }
    }
}

impl Cursor<'_> {
    fn bump_one(&mut self) {
        self.bump();
    }
}

/// Parse the `.@label` attribute suffix. Returns the attributes plus whether an
/// `@entry` marker was seen (the caller decides what to do with it).
pub(crate) fn parse_attrs(c: &mut Cursor) -> Result<(AttrList, bool), ParseError> {
    let mut attrs = AttrList::new();
    let mut entry = false;
    while c.starts_with(".@") {
        c.advance_str(".@");
        let label = c.take_while(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
        if label.is_empty() {
            return Err(c.error("expected an attribute label after '.@'"));
        }
        if label == "entry" {
            entry = true;
        } else {
            attrs.push(Attr::from_label(label));
        }
    }
    Ok((attrs, entry))
}

/// Parse a full inline UW: identity + attributes. Returns the identity, its
/// attributes, and whether it carried the `@entry` marker.
pub(crate) fn parse_uw(c: &mut Cursor) -> Result<(Uci, AttrList, bool), ParseError> {
    let uci = parse_uci(c)?;
    let (attrs, entry) = parse_attrs(c)?;
    Ok((uci, attrs, entry))
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Write a UCI to its canonical text form.
pub(crate) fn write_uci(uci: &Uci, out: &mut String) {
    match uci {
        Uci::Null => out.push_str("00"),
        Uci::Temporary(s) => {
            out.push('"');
            for ch in s.chars() {
                if ch == '\\' || ch == '"' {
                    out.push('\\');
                }
                out.push(ch);
            }
            out.push('"');
        }
        Uci::Ucl {
            authority: None,
            id,
        } => out.push_str(&id.to_string()),
        Uci::Ucl {
            authority: Some(a),
            id,
        } => {
            out.push_str("ucl://");
            out.push_str(a);
            out.push('/');
            out.push_str(&id.to_string());
        }
        Uci::Ucn { root, suffix, .. } => {
            out.push_str(root);
            if let Some(s) = suffix {
                out.push('(');
                out.push_str(s.relation.as_str());
                out.push('>');
                out.push_str(&s.word);
                out.push(')');
            }
        }
    }
}

/// Write an inline UW (identity + attributes), appending the `@entry` marker if
/// `is_entry`.
pub(crate) fn write_uw(uw: &unl_core::Uw, is_entry: bool, out: &mut String) {
    write_uci(&uw.uci, out);
    for attr in uw.attributes.iter() {
        out.push_str(".@");
        out.push_str(attr.as_label());
    }
    if is_entry {
        out.push_str(".@entry");
    }
}
