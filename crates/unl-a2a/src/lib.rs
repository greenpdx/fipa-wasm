//! # unl-a2a
//!
//! The payoff (`~/SOURCES_MANIFEST.md` §7): UNL as the **content language** for
//! agent-to-agent messaging. The content of a message is a [`UnlGraph`] — its
//! *meaning*, not surface text — so it survives translation, can be reasoned
//! over, and can be **verified before execution**.
//!
//! - [`A2aMessage`] — a semantic message: a UNL graph plus an addressing /
//!   conversation envelope.
//! - [`A2aCodec`] — wire encode/decode. [`UnlWireCodec`] is the compact default
//!   (small header + UNL list-format content); [`JsonCodec`] is the interop form
//!   for non-UNL agents.
//! - [`A2aVerify`] — the security boundary: an agent never acts on intent it
//!   could not validate against its local KB. A nonsensical request fails
//!   [`A2aVerifier::verify`] deterministically instead of being half-understood.

mod codec;

pub use codec::{JsonCodec, UnlWireCodec};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use unl_core::UnlGraph;
use unl_kb::KnowledgeBase;
use unl_validator::{Diagnostic, Severity, Validate};

/// An agent address.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub SmolStr);

/// A conversation correlation id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub SmolStr);

/// A message id (for `reply_to` threading).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub SmolStr);

impl<T: Into<SmolStr>> From<T> for AgentId {
    fn from(s: T) -> Self {
        AgentId(s.into())
    }
}
impl<T: Into<SmolStr>> From<T> for ConversationId {
    fn from(s: T) -> Self {
        ConversationId(s.into())
    }
}
impl<T: Into<SmolStr>> From<T> for MessageId {
    fn from(s: T) -> Self {
        MessageId(s.into())
    }
}

/// A semantic message between agents. The content is a UNL graph; the envelope
/// carries addressing and conversation state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct A2aMessage {
    pub sender: AgentId,
    pub receiver: AgentId,
    pub conversation_id: ConversationId,
    /// The semantic content as a UNL graph.
    pub content: UnlGraph,
    /// Optional original NL rendering, for human audit / logging.
    pub gloss: Option<String>,
    pub reply_to: Option<MessageId>,
}

impl A2aMessage {
    /// A minimal message (no gloss, not a reply).
    pub fn new(
        sender: impl Into<AgentId>,
        receiver: impl Into<AgentId>,
        conversation_id: impl Into<ConversationId>,
        content: UnlGraph,
    ) -> Self {
        A2aMessage {
            sender: sender.into(),
            receiver: receiver.into(),
            conversation_id: conversation_id.into(),
            content,
            gloss: None,
            reply_to: None,
        }
    }
}

/// Encode/decode A2A messages to/from the wire.
pub trait A2aCodec {
    fn encode(&self, msg: &A2aMessage) -> Vec<u8>;
    fn decode(&self, bytes: &[u8]) -> Result<A2aMessage, A2aError>;
}

/// Verify that a received message is well-formed and its content validates
/// against the local KB before the agent acts on it.
pub trait A2aVerify {
    /// `Ok(())` if the content is well-formed; otherwise the blocking error
    /// diagnostics. Warnings (e.g. unknown concepts) do not block.
    fn verify(&self, msg: &A2aMessage, kb: &dyn KnowledgeBase) -> Result<(), Vec<Diagnostic>>;
}

/// The default verifier: runs `unl-validator` over the content and blocks on any
/// error-severity diagnostic (dangling references, incompatible attributes, …).
#[derive(Default)]
pub struct A2aVerifier;

impl A2aVerify for A2aVerifier {
    fn verify(&self, msg: &A2aMessage, kb: &dyn KnowledgeBase) -> Result<(), Vec<Diagnostic>> {
        let errors: Vec<Diagnostic> = msg
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
}

#[derive(Debug, thiserror::Error)]
pub enum A2aError {
    #[error("malformed message: {0}")]
    Malformed(String),
    #[error("invalid UTF-8 in wire message")]
    Utf8,
    #[error("content parse error: {0}")]
    Content(#[from] unl_parser::ParseError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod verify_tests {
    use super::*;
    use unl_core::{Relation, RelationTag, Uci, UnlGraph, Uw};
    use unl_kb::MemKb;
    use unl_validator::DiagCode;

    fn kb() -> MemKb {
        MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap()
    }

    #[test]
    fn verify_accepts_well_formed_content() {
        // cat icl animal — resolvable and ontologically consistent.
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("cat")));
        g.insert_node("02", Uw::new(Uci::ucn("animal")));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Icl, "01".into(), "02".into()));

        let msg = A2aMessage::new("alice", "bob", "c-1", g);
        assert!(A2aVerifier.verify(&msg, &kb()).is_ok());
    }

    #[test]
    fn verify_rejects_malformed_content() {
        // Relation references an undeclared node => DanglingReference (Error).
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("cat")));
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "99".into()));

        let msg = A2aMessage::new("alice", "bob", "c-1", g);
        let errors = A2aVerifier.verify(&msg, &kb()).unwrap_err();
        assert!(errors.iter().any(|d| d.code == DiagCode::DanglingReference));
    }

    #[test]
    fn verify_is_object_safe() {
        let v: &dyn A2aVerify = &A2aVerifier;
        let msg = A2aMessage::new("a", "b", "c", UnlGraph::new());
        assert!(v.verify(&msg, &kb()).is_ok()); // empty content has no errors
    }
}
