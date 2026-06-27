//! # unl-fipa
//!
//! UNL slots *under* a FIPA ACL performative (`~/SOURCES_MANIFEST.md` §8). FIPA
//! handles the speech act (`REQUEST`, `INFORM`, `PROPOSE`); UNL carries the
//! content. This is exactly the `:content-language` slot FIPA ACL was designed
//! for, and it connects to the `fipa-wasm` / AGNTCon work.
//!
//! ```text
//!   AclMessage          ← performative envelope (FIPA): WHAT speech act
//!     └── UnlGraph       ← semantic content (UNL):       WHAT is meant
//!           └── Uci      ← grounded concepts (KB):        WHICH concepts
//! ```
//!
//! An agent receiving a [`Performative::Request`] whose UNL content fails
//! [`AclMessage::verify_content`] replies [`AclMessage::not_understood`] —
//! cleanly, by construction. That is the property neither raw-NL nor raw-JSON
//! A2A can offer.

mod performative;
mod sexpr;

pub use performative::{Performative, UnknownPerformative};
// Addressing types are shared with the A2A layer.
pub use unl_a2a::{AgentId, ConversationId};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use unl_core::UnlGraph;
use unl_kb::KnowledgeBase;
use unl_validator::{Diagnostic, Severity, Validate};

/// A FIPA ACL message whose content language is UNL.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AclMessage {
    pub performative: Performative,
    pub sender: AgentId,
    pub receiver: Vec<AgentId>,
    pub reply_with: Option<SmolStr>,
    pub in_reply_to: Option<SmolStr>,
    pub conversation_id: Option<ConversationId>,
    pub protocol: Option<SmolStr>,
    /// The content; `content-language` is fixed to `UNL`.
    pub content: UnlGraph,
}

impl AclMessage {
    pub const CONTENT_LANGUAGE: &'static str = "UNL";

    /// A minimal message with one receiver and no optional parameters.
    pub fn new(
        performative: Performative,
        sender: impl Into<AgentId>,
        receiver: impl Into<AgentId>,
        content: UnlGraph,
    ) -> Self {
        AclMessage {
            performative,
            sender: sender.into(),
            receiver: vec![receiver.into()],
            reply_with: None,
            in_reply_to: None,
            conversation_id: None,
            protocol: None,
            content,
        }
    }

    /// Validate the content graph against a knowledge base. `Ok(())` if
    /// well-formed; otherwise the blocking error diagnostics. (Warnings, e.g.
    /// unknown concepts, do not block — the same boundary as `unl-a2a`.)
    pub fn verify_content(&self, kb: &dyn KnowledgeBase) -> Result<(), Vec<Diagnostic>> {
        let errors: Vec<Diagnostic> = self
            .content
            .validate(kb)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// The `not-understood` reply an agent sends when it cannot validate this
    /// message — threaded back to the sender, echoing the conversation.
    pub fn not_understood(&self, from: impl Into<AgentId>) -> AclMessage {
        AclMessage {
            performative: Performative::NotUnderstood,
            sender: from.into(),
            receiver: vec![self.sender.clone()],
            reply_with: None,
            in_reply_to: self.reply_with.clone(),
            conversation_id: self.conversation_id.clone(),
            protocol: self.protocol.clone(),
            content: UnlGraph::new(),
        }
    }

    /// Serialize to the standard FIPA ACL string form, with the UNL graph as the
    /// `:content` (UNL list format, quoted).
    pub fn to_fipa_string(&self) -> String {
        let mut out = String::new();
        out.push('(');
        out.push_str(self.performative.as_str());
        out.push('\n');

        out.push_str(" :sender ");
        write_agent(&self.sender, &mut out);
        out.push('\n');

        out.push_str(" :receiver (set");
        for r in &self.receiver {
            out.push(' ');
            write_agent(r, &mut out);
        }
        out.push_str(")\n");

        out.push_str(" :content \"");
        escape_into(&unl_parser::serialize_list(&self.content), &mut out);
        out.push_str("\"\n");

        out.push_str(" :language ");
        out.push_str(Self::CONTENT_LANGUAGE);
        out.push('\n');

        if let Some(p) = &self.protocol {
            out.push_str(" :protocol ");
            out.push_str(p);
            out.push('\n');
        }
        if let Some(c) = &self.conversation_id {
            out.push_str(" :conversation-id ");
            out.push_str(&c.0);
            out.push('\n');
        }
        if let Some(r) = &self.reply_with {
            out.push_str(" :reply-with ");
            out.push_str(r);
            out.push('\n');
        }
        if let Some(r) = &self.in_reply_to {
            out.push_str(" :in-reply-to ");
            out.push_str(r);
            out.push('\n');
        }
        out.push(')');
        out
    }

    /// Parse a FIPA ACL string (with a UNL `:content`) back into a message.
    pub fn from_fipa_string(s: &str) -> Result<Self, FipaError> {
        sexpr::parse_acl(s)
    }
}

fn write_agent(agent: &AgentId, out: &mut String) {
    out.push_str("(agent-identifier :name ");
    out.push_str(&agent.0);
    out.push(')');
}

fn escape_into(s: &str, out: &mut String) {
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FipaError {
    #[error("FIPA syntax error: {0}")]
    Syntax(String),
    #[error("unknown performative: {0}")]
    Performative(String),
    #[error("missing required ACL parameter: {0}")]
    Missing(&'static str),
    #[error("content (UNL) parse error: {0}")]
    Content(#[from] unl_parser::ParseError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use unl_core::{Relation, RelationTag, Uci, Uw};
    use unl_kb::MemKb;
    use unl_validator::DiagCode;

    fn kb() -> MemKb {
        MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap()
    }

    fn killed_graph() -> UnlGraph {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("kill")));
        g.insert_node("02", Uw::new(Uci::ucn("Peter")));
        g.insert_node("03", Uw::new(Uci::ucn("John")));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
        g.add_relation(Relation::between(RelationTag::Obj, "01".into(), "03".into()));
        g
    }

    #[test]
    fn content_language_is_unl() {
        assert_eq!(AclMessage::CONTENT_LANGUAGE, "UNL");
    }

    #[test]
    fn performative_str_roundtrips() {
        for p in Performative::ALL {
            assert_eq!(p.as_str().parse::<Performative>().unwrap(), p);
        }
        assert!("frobnicate".parse::<Performative>().is_err());
    }

    #[test]
    fn fipa_string_roundtrip_full() {
        let msg = AclMessage {
            performative: Performative::Request,
            sender: "alice".into(),
            receiver: vec!["bob".into(), "carol".into()],
            reply_with: Some("r1".into()),
            in_reply_to: Some("q0".into()),
            conversation_id: Some("c-42".into()),
            protocol: Some("fipa-request".into()),
            content: killed_graph(),
        };
        let s = msg.to_fipa_string();
        assert!(s.starts_with("(request"));
        assert!(s.contains(":language UNL"));
        assert_eq!(AclMessage::from_fipa_string(&s).unwrap(), msg);
    }

    #[test]
    fn fipa_string_roundtrip_minimal() {
        let msg = AclMessage::new(Performative::Inform, "a", "b", killed_graph());
        let s = msg.to_fipa_string();
        assert_eq!(AclMessage::from_fipa_string(&s).unwrap(), msg);
    }

    #[test]
    fn request_with_bad_content_yields_not_understood() {
        // A request whose content references an undeclared node.
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("do")));
        g.add_relation(Relation::between(RelationTag::Obj, "01".into(), "99".into()));
        let request = AclMessage {
            performative: Performative::Request,
            sender: "alice".into(),
            receiver: vec!["bob".into()],
            reply_with: Some("r1".into()),
            in_reply_to: None,
            conversation_id: Some("c-7".into()),
            protocol: Some("fipa-request".into()),
            content: g,
        };

        // The receiver cannot validate it...
        let errors = request.verify_content(&kb()).unwrap_err();
        assert!(errors.iter().any(|d| d.code == DiagCode::DanglingReference));

        // ...so it replies not-understood, by construction.
        let reply = request.not_understood("bob");
        assert_eq!(reply.performative, Performative::NotUnderstood);
        assert_eq!(reply.sender, "bob".into());
        assert_eq!(reply.receiver, vec![AgentId::from("alice")]);
        assert_eq!(reply.in_reply_to.as_deref(), Some("r1"));
        assert_eq!(reply.conversation_id, Some("c-7".into()));
    }

    #[test]
    fn valid_request_verifies() {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("cat")));
        g.insert_node("02", Uw::new(Uci::ucn("animal")));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Icl, "01".into(), "02".into()));
        let msg = AclMessage::new(Performative::Inform, "a", "b", g);
        assert!(msg.verify_content(&kb()).is_ok());
    }
}
