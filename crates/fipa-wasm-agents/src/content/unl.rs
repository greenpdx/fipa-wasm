//! UNL content-language bridge.
//!
//! Wires the `unl-*` stack (and `unl-fipa`) into the runtime's wire message
//! [`proto::AclMessage`]. UNL becomes a `content-language` agents can carry:
//! the graph is serialized into the message `content` bytes with
//! `language = "UNL"`, and a received message can be **verified against the
//! local knowledge base before the agent acts on it** — failing into a
//! `not-understood` reply by construction.
//!
//! ```ignore
//! use fipa_wasm_agents::content::unl;
//!
//! // Attach semantic content:
//! unl::set_unl_content(&mut msg, &graph);
//!
//! // On receipt — the security boundary:
//! match unl::verify(&msg, &kb) {
//!     Ok(()) => act_on(unl::unl_graph(&msg)?),
//!     Err(_) => send(unl::not_understood(&msg, my_name)),
//! }
//!
//! // Interop with external (non-wasm) UNL agents over FIPA-ACL strings:
//! let wire = unl::to_fipa_string(&msg)?;
//! let incoming = unl::from_fipa_string(&wire)?;
//! ```

use crate::proto::{AclMessage, AgentId, Performative};
use std::collections::HashMap;
use unl_core::UnlGraph;
use unl_kb::KnowledgeBase;
use unl_validator::Diagnostic;

/// The content-language tag agents set on UNL messages (`AclMessage.language`).
pub const CONTENT_LANGUAGE: &str = "UNL";

#[derive(Debug, thiserror::Error)]
pub enum UnlError {
    #[error("message content language is not UNL")]
    NotUnl,
    #[error("UNL content is not valid UTF-8")]
    Utf8,
    #[error("unparseable UNL content: {0}")]
    Parse(#[from] unl_parser::ParseError),
    #[error("FIPA string error: {0}")]
    Fipa(String),
    #[error("performative has no UNL/FIPA equivalent")]
    Performative,
    #[error("content failed verification: {} error diagnostic(s)", .0.len())]
    Invalid(Vec<Diagnostic>),
}

/// True if the message declares UNL as its content language.
pub fn is_unl(msg: &AclMessage) -> bool {
    msg.language.as_deref() == Some(CONTENT_LANGUAGE)
}

/// Attach a UNL graph as the content (list format, UTF-8, `language = "UNL"`).
pub fn set_unl_content(msg: &mut AclMessage, graph: &UnlGraph) {
    msg.content = unl_parser::serialize_list(graph).into_bytes();
    msg.language = Some(CONTENT_LANGUAGE.to_string());
    msg.encoding = Some("utf-8".to_string());
}

/// Parse the UNL graph from a message's content.
pub fn unl_graph(msg: &AclMessage) -> Result<UnlGraph, UnlError> {
    if !is_unl(msg) {
        return Err(UnlError::NotUnl);
    }
    let text = std::str::from_utf8(&msg.content).map_err(|_| UnlError::Utf8)?;
    Ok(unl_parser::parse_sentence(text)?)
}

/// Verify a UNL-content message against the local KB — the security boundary.
/// Routes through the `unl-fipa` envelope so the runtime shares one path.
pub fn verify(msg: &AclMessage, kb: &dyn KnowledgeBase) -> Result<(), UnlError> {
    to_unl_fipa(msg)?
        .verify_content(kb)
        .map_err(UnlError::Invalid)
}

/// The `not-understood` reply an agent sends when it cannot validate a message —
/// threaded back to the original sender, echoing the conversation.
pub fn not_understood(msg: &AclMessage, from: &str) -> AclMessage {
    AclMessage {
        message_id: new_id(),
        performative: Performative::NotUnderstood as i32,
        sender: Some(agent_id(from)),
        receivers: msg.sender.clone().into_iter().collect(),
        reply_to: None,
        protocol: msg.protocol,
        conversation_id: msg.conversation_id.clone(),
        in_reply_to: msg.reply_with.clone(),
        reply_with: None,
        reply_by: None,
        language: Some(CONTENT_LANGUAGE.to_string()),
        encoding: Some("utf-8".to_string()),
        ontology: None,
        content: Vec::new(),
        user_properties: HashMap::new(),
    }
}

/// Render a UNL message in the standard FIPA-ACL string form via `unl-fipa`,
/// for interop with external UNL agents.
pub fn to_fipa_string(msg: &AclMessage) -> Result<String, UnlError> {
    Ok(to_unl_fipa(msg)?.to_fipa_string())
}

/// Parse an external FIPA-ACL string (UNL content) into a runtime message.
pub fn from_fipa_string(s: &str) -> Result<AclMessage, UnlError> {
    let envelope =
        unl_fipa::AclMessage::from_fipa_string(s).map_err(|e| UnlError::Fipa(e.to_string()))?;
    Ok(from_unl_fipa(&envelope))
}

/// Convert a runtime ACL message to the `unl-fipa` envelope (parsing content).
/// `protocol` stays in the runtime's proto envelope; it is not carried into the
/// UNL/FIPA-SL form here.
pub fn to_unl_fipa(msg: &AclMessage) -> Result<unl_fipa::AclMessage, UnlError> {
    let content = unl_graph(msg)?;
    let performative = proto_to_unl_perf(
        Performative::try_from(msg.performative).unwrap_or(Performative::Unspecified),
    )?;
    Ok(unl_fipa::AclMessage {
        performative,
        sender: unl_fipa::AgentId::from(
            msg.sender.as_ref().map(|a| a.name.as_str()).unwrap_or(""),
        ),
        receiver: msg
            .receivers
            .iter()
            .map(|a| unl_fipa::AgentId::from(a.name.as_str()))
            .collect(),
        reply_with: msg.reply_with.clone().map(Into::into),
        in_reply_to: msg.in_reply_to.clone().map(Into::into),
        conversation_id: msg.conversation_id.clone().map(unl_fipa::ConversationId::from),
        protocol: None,
        content,
    })
}

/// Convert a `unl-fipa` envelope back to a runtime ACL message (serializing
/// content). A fresh `message_id` is minted.
pub fn from_unl_fipa(env: &unl_fipa::AclMessage) -> AclMessage {
    AclMessage {
        message_id: new_id(),
        performative: unl_to_proto_perf(env.performative) as i32,
        sender: Some(agent_id(&env.sender.0)),
        receivers: env.receiver.iter().map(|a| agent_id(&a.0)).collect(),
        reply_to: None,
        protocol: None,
        conversation_id: env.conversation_id.as_ref().map(|c| c.0.to_string()),
        in_reply_to: env.in_reply_to.as_ref().map(|s| s.to_string()),
        reply_with: env.reply_with.as_ref().map(|s| s.to_string()),
        reply_by: None,
        language: Some(CONTENT_LANGUAGE.to_string()),
        encoding: Some("utf-8".to_string()),
        ontology: None,
        content: unl_parser::serialize_list(&env.content).into_bytes(),
        user_properties: HashMap::new(),
    }
}

fn agent_id(name: &str) -> AgentId {
    AgentId {
        name: name.to_string(),
        addresses: Vec::new(),
        resolvers: Vec::new(),
    }
}

fn new_id() -> String {
    format!("unl-{}", uuid::Uuid::new_v4())
}

fn proto_to_unl_perf(p: Performative) -> Result<unl_fipa::Performative, UnlError> {
    use unl_fipa::Performative as U;
    Ok(match p {
        Performative::AcceptProposal => U::AcceptProposal,
        Performative::Agree => U::Agree,
        Performative::Cancel => U::Cancel,
        Performative::Cfp => U::Cfp,
        Performative::Confirm => U::Confirm,
        Performative::Disconfirm => U::Disconfirm,
        Performative::Failure => U::Failure,
        // FIPA's InformDone/InformResult collapse to Inform in the 22-set.
        Performative::Inform | Performative::InformDone | Performative::InformResult => U::Inform,
        Performative::InformIf => U::InformIf,
        Performative::InformRef => U::InformRef,
        Performative::NotUnderstood => U::NotUnderstood,
        Performative::Propagate => U::Propagate,
        Performative::Propose => U::Propose,
        Performative::Proxy => U::Proxy,
        Performative::QueryIf => U::QueryIf,
        Performative::QueryRef => U::QueryRef,
        Performative::Refuse => U::Refuse,
        Performative::RejectProposal => U::RejectProposal,
        Performative::Request => U::Request,
        Performative::RequestWhen => U::RequestWhen,
        Performative::RequestWhenever => U::RequestWhenever,
        Performative::Subscribe => U::Subscribe,
        Performative::Unspecified => return Err(UnlError::Performative),
    })
}

fn unl_to_proto_perf(p: unl_fipa::Performative) -> Performative {
    use unl_fipa::Performative as U;
    match p {
        U::AcceptProposal => Performative::AcceptProposal,
        U::Agree => Performative::Agree,
        U::Cancel => Performative::Cancel,
        U::Cfp => Performative::Cfp,
        U::Confirm => Performative::Confirm,
        U::Disconfirm => Performative::Disconfirm,
        U::Failure => Performative::Failure,
        U::Inform => Performative::Inform,
        U::InformIf => Performative::InformIf,
        U::InformRef => Performative::InformRef,
        U::NotUnderstood => Performative::NotUnderstood,
        U::Propagate => Performative::Propagate,
        U::Propose => Performative::Propose,
        U::Proxy => Performative::Proxy,
        U::QueryIf => Performative::QueryIf,
        U::QueryRef => Performative::QueryRef,
        U::Refuse => Performative::Refuse,
        U::RejectProposal => Performative::RejectProposal,
        U::Request => Performative::Request,
        U::RequestWhen => Performative::RequestWhen,
        U::RequestWhenever => Performative::RequestWhenever,
        U::Subscribe => Performative::Subscribe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unl_core::{Relation, RelationTag, Uci, Uw};
    use unl_kb::MemKb;

    fn kb() -> MemKb {
        MemKb::from_toml(include_str!("../../../../data/kb-seed/memkb-fixture.toml")).unwrap()
    }

    fn base_msg(perf: Performative, sender: &str, receiver: &str) -> AclMessage {
        AclMessage {
            message_id: "m1".to_string(),
            performative: perf as i32,
            sender: Some(agent_id(sender)),
            receivers: vec![agent_id(receiver)],
            reply_to: None,
            protocol: None,
            conversation_id: Some("c-1".to_string()),
            in_reply_to: None,
            reply_with: Some("r1".to_string()),
            reply_by: None,
            language: None,
            encoding: None,
            ontology: None,
            content: Vec::new(),
            user_properties: HashMap::new(),
        }
    }

    fn cat_icl_animal() -> UnlGraph {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("cat")));
        g.insert_node("02", Uw::new(Uci::ucn("animal")));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Icl, "01".into(), "02".into()));
        g
    }

    fn dangling() -> UnlGraph {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("do")));
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "99".into()));
        g
    }

    fn first_tag(msg: &AclMessage) -> RelationTag {
        unl_graph(msg).unwrap().relations[0].tag
    }

    #[test]
    fn attach_and_read_unl_content() {
        let mut m = base_msg(Performative::Request, "alice", "bob");
        assert!(!is_unl(&m));
        set_unl_content(&mut m, &cat_icl_animal());
        assert!(is_unl(&m));
        assert_eq!(first_tag(&m), RelationTag::Icl);
    }

    #[test]
    fn verify_accepts_valid_content() {
        let mut m = base_msg(Performative::Inform, "a", "b");
        set_unl_content(&mut m, &cat_icl_animal());
        assert!(verify(&m, &kb()).is_ok());
    }

    #[test]
    fn unverifiable_request_yields_not_understood() {
        let mut m = base_msg(Performative::Request, "alice", "bob");
        set_unl_content(&mut m, &dangling());
        assert!(matches!(verify(&m, &kb()), Err(UnlError::Invalid(_))));

        let nu = not_understood(&m, "bob");
        assert_eq!(nu.performative, Performative::NotUnderstood as i32);
        assert_eq!(nu.sender.unwrap().name, "bob");
        assert_eq!(nu.receivers[0].name, "alice");
        assert_eq!(nu.in_reply_to.as_deref(), Some("r1"));
        assert_eq!(nu.conversation_id.as_deref(), Some("c-1"));
    }

    #[test]
    fn fipa_string_interop() {
        let mut m = base_msg(Performative::Request, "alice", "bob");
        set_unl_content(&mut m, &cat_icl_animal());
        let s = to_fipa_string(&m).unwrap();
        assert!(s.starts_with("(request"));
        assert!(s.contains(":language UNL"));

        let back = from_fipa_string(&s).unwrap();
        assert_eq!(back.performative, Performative::Request as i32);
        assert_eq!(back.sender.as_ref().unwrap().name, "alice");
        assert_eq!(first_tag(&back), RelationTag::Icl);
    }

    #[test]
    fn runtime_roundtrip_via_unl_fipa() {
        let mut m = base_msg(Performative::Inform, "alice", "bob");
        set_unl_content(&mut m, &cat_icl_animal());
        let back = from_unl_fipa(&to_unl_fipa(&m).unwrap());
        assert_eq!(back.performative, m.performative);
        assert_eq!(back.sender.as_ref().unwrap().name, "alice");
        assert_eq!(back.receivers[0].name, "bob");
        assert_eq!(back.conversation_id, m.conversation_id);
        assert_eq!(first_tag(&back), RelationTag::Icl);
    }
}
