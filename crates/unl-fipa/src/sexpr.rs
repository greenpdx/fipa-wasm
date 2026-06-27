//! A small s-expression reader for the FIPA ACL string representation, and the
//! `from_fipa_string` parser built on it.

use crate::{AclMessage, FipaError, Performative};
use unl_a2a::{AgentId, ConversationId};

enum Tok {
    Open,
    Close,
    Str(String),
    Atom(String),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, FipaError> {
    let mut toks = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' => toks.push(Tok::Open),
            ')' => toks.push(Tok::Close),
            '"' => {
                let mut buf = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => match chars.next() {
                            Some(esc) => buf.push(esc),
                            None => return Err(FipaError::Syntax("dangling escape".into())),
                        },
                        Some('"') => break,
                        Some(ch) => buf.push(ch),
                        None => return Err(FipaError::Syntax("unterminated string".into())),
                    }
                }
                toks.push(Tok::Str(buf));
            }
            c if c.is_whitespace() => {}
            _ => {
                let mut buf = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n == '(' || n == ')' || n == '"' || n.is_whitespace() {
                        break;
                    }
                    buf.push(n);
                    chars.next();
                }
                toks.push(Tok::Atom(buf));
            }
        }
    }
    Ok(toks)
}

/// Parsed s-expression.
enum Sexpr {
    Atom(String),
    Str(String),
    List(Vec<Sexpr>),
}

fn parse(toks: &[Tok], pos: &mut usize) -> Result<Sexpr, FipaError> {
    match toks.get(*pos) {
        Some(Tok::Open) => {
            *pos += 1;
            let mut items = Vec::new();
            loop {
                match toks.get(*pos) {
                    Some(Tok::Close) => {
                        *pos += 1;
                        return Ok(Sexpr::List(items));
                    }
                    None => return Err(FipaError::Syntax("unclosed list".into())),
                    _ => items.push(parse(toks, pos)?),
                }
            }
        }
        Some(Tok::Str(s)) => {
            *pos += 1;
            Ok(Sexpr::Str(s.clone()))
        }
        Some(Tok::Atom(a)) => {
            *pos += 1;
            Ok(Sexpr::Atom(a.clone()))
        }
        Some(Tok::Close) => Err(FipaError::Syntax("unexpected ')'".into())),
        None => Err(FipaError::Syntax("unexpected end of input".into())),
    }
}

fn agent_name(sexpr: &Sexpr) -> Result<AgentId, FipaError> {
    // (agent-identifier :name <name> ...)
    if let Sexpr::List(items) = sexpr
        && matches!(items.first(), Some(Sexpr::Atom(h)) if h == "agent-identifier")
    {
        for pair in items.windows(2) {
            if matches!(&pair[0], Sexpr::Atom(k) if k == ":name")
                && let Sexpr::Atom(name) = &pair[1]
            {
                return Ok(AgentId::from(name.as_str()));
            }
        }
    }
    Err(FipaError::Syntax("expected (agent-identifier :name ...)".into()))
}

/// Parse the FIPA ACL string form into an [`AclMessage`].
pub(crate) fn parse_acl(input: &str) -> Result<AclMessage, FipaError> {
    let toks = tokenize(input)?;
    let mut pos = 0;
    let top = parse(&toks, &mut pos)?;

    let Sexpr::List(items) = top else {
        return Err(FipaError::Syntax("top level must be a list".into()));
    };
    let mut it = items.iter();
    let performative: Performative = match it.next() {
        Some(Sexpr::Atom(p)) => p.parse().map_err(|_| FipaError::Performative(p.clone()))?,
        _ => return Err(FipaError::Syntax("missing performative".into())),
    };

    let mut sender = None;
    let mut receiver = Vec::new();
    let mut content_text = None;
    let mut reply_with = None;
    let mut in_reply_to = None;
    let mut conversation_id = None;
    let mut protocol = None;

    while let Some(key) = it.next() {
        let Sexpr::Atom(key) = key else {
            return Err(FipaError::Syntax("expected :keyword".into()));
        };
        let value = it.next().ok_or_else(|| FipaError::Syntax(format!("{key} has no value")))?;
        match key.as_str() {
            ":sender" => sender = Some(agent_name(value)?),
            ":receiver" => {
                // (set (agent-identifier ...) ...)
                if let Sexpr::List(set) = value {
                    for a in set.iter().skip(1) {
                        receiver.push(agent_name(a)?);
                    }
                }
            }
            ":content" => {
                if let Sexpr::Str(s) = value {
                    content_text = Some(s.clone());
                }
            }
            ":language" => {} // fixed to UNL; ignore on parse
            ":protocol" => protocol = atom(value),
            ":conversation-id" => conversation_id = atom(value).map(ConversationId),
            ":reply-with" => reply_with = atom(value),
            ":in-reply-to" => in_reply_to = atom(value),
            _ => {} // tolerate unknown ACL parameters
        }
    }

    let content_text = content_text.ok_or(FipaError::Missing("content"))?;
    let content = unl_parser::parse_list(&content_text)?;

    Ok(AclMessage {
        performative,
        sender: sender.ok_or(FipaError::Missing("sender"))?,
        receiver,
        reply_with,
        in_reply_to,
        conversation_id,
        protocol,
        content,
    })
}

fn atom(sexpr: &Sexpr) -> Option<smol_str::SmolStr> {
    match sexpr {
        Sexpr::Atom(a) => Some(a.as_str().into()),
        _ => None,
    }
}
