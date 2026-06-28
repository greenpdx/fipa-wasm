//! UNL content-language bridge.
//!
//! Wires the `unl-*` stack (and `unl-fipa`) into the runtime's wire message
//! [`proto::AclMessage`]. UNL becomes a `content-language` agents can carry: the
//! graph is serialized into the message `content` bytes with `language = "UNL"`.
//!
//! [`UnlVerifier`] is the UNL implementation of the FIPA layer's
//! [`crate::content::verify::ContentVerifier`] seam: it accepts UNL content only
//! when it lies entirely within the **agent's [`Vocabulary`]** (and is
//! structurally sound). Non-UNL content passes through. An out-of-vocabulary
//! message is `not-understood` — the agent literally has no word for it.
//!
//! ```ignore
//! use fipa_wasm_agents::content::unl::UnlVerifier;
//!
//! // The agent ships its own compact vocabulary:
//! let verifier = UnlVerifier::new(my_vocabulary);
//! supervisor.with_content_verifier(Arc::new(verifier));
//!
//! // Attach / read semantic content, or interop over FIPA-ACL strings:
//! unl::set_unl_content(&mut msg, &graph);
//! let wire = unl::to_fipa_string(&msg)?;
//! ```

use crate::content::block::{BlockFile, TAG_DATA, TAG_UNL};
use crate::content::verify::ContentVerifier;
use crate::proto::{AclMessage, AgentId, Performative};
use crate::content::verify::OutboundPackager;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use unl_core::UnlGraph;
use unl_kb::Vocabulary;
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
    #[error("malformed message blocks: {0}")]
    Block(String),
}

/// A [`ContentVerifier`] that accepts UNL content only when it lies entirely
/// within the agent's [`Vocabulary`] and is structurally sound. Non-UNL content
/// is not this verifier's concern and passes through.
pub struct UnlVerifier {
    vocab: Vocabulary,
}

impl UnlVerifier {
    pub fn new(vocab: Vocabulary) -> Self {
        UnlVerifier { vocab }
    }

    /// Build a verifier from an agent bundle's `UNL ` block — the agent's
    /// vocabulary rules (data) that the node reads and turns into a verifier
    /// (process). Returns `None` if the bundle has no UNL block (the agent
    /// declares no UNL rules), `Some(Err)` if the block is malformed.
    pub fn from_bundle(bundle: &BlockFile) -> Option<Result<UnlVerifier, serde_json::Error>> {
        Some(vocabulary_from_bundle(bundle)?.map(UnlVerifier::new))
    }
}

/// Deserialize an agent's [`Vocabulary`] from its `UNL ` block. `None` if the
/// bundle has no UNL block.
pub fn vocabulary_from_bundle(bundle: &BlockFile) -> Option<Result<Vocabulary, serde_json::Error>> {
    let bytes = bundle.get(TAG_UNL)?;
    Some(serde_json::from_slice::<Vocabulary>(bytes))
}

/// Serialize a vocabulary into the bytes of an agent's `UNL ` block (the
/// authoring side — produces the rules an agent ships).
pub fn vocabulary_block(vocab: &Vocabulary) -> Vec<u8> {
    serde_json::to_vec(vocab).expect("Vocabulary serializes")
}

/// The UNL outbound packager: validate the agent's emitted UNL against the
/// receiver's vocabulary (in the shared registry), then package it. The UNL
/// implementation of the content-agnostic [`OutboundPackager`] seam.
pub struct UnlPackager {
    registry: Arc<RwLock<VocabRegistry>>,
}

impl UnlPackager {
    pub fn new(registry: Arc<RwLock<VocabRegistry>>) -> Self {
        UnlPackager { registry }
    }
}

impl OutboundPackager for UnlPackager {
    fn package(
        &self,
        sender: &str,
        receiver: &str,
        unl: &[u8],
        body: &[u8],
    ) -> Result<AclMessage, String> {
        let text = std::str::from_utf8(unl).map_err(|_| "outgoing UNL is not UTF-8".to_string())?;
        let graph = unl_parser::parse_sentence(text).map_err(|e| e.to_string())?;
        let registry = self.registry.read().map_err(|_| "vocab registry poisoned".to_string())?;
        package_outbound(sender, receiver, &graph, body, &registry).map_err(|diags| {
            format!("receiver '{receiver}' would not understand ({} issue(s))", diags.len())
        })
    }
}

impl ContentVerifier for UnlVerifier {
    fn verify(&self, msg: &AclMessage) -> Result<(), String> {
        if !is_unl(msg) {
            return Ok(());
        }
        let graph = unl_graph(msg).map_err(|e| e.to_string())?;
        unl_validator::verify_vocabulary(&graph, &self.vocab).map_err(|diags| {
            let terms = diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ");
            format!("not understood ({} issue(s)): {terms}", diags.len())
        })
    }

    fn sanitize(
        &self,
        msg: &AclMessage,
    ) -> Result<Option<crate::content::verify::Decoded>, String> {
        self.verify(msg)?;
        if !is_unl(msg) {
            return Ok(None);
        }
        let graph = unl_graph(msg).map_err(|e| e.to_string())?;
        Ok(Some(crate::content::verify::Decoded {
            unl: unl_parser::serialize_list(&graph).into_bytes(),
            body: message_data(msg),
        }))
    }
}

/// True if the message declares UNL as its content language.
pub fn is_unl(msg: &AclMessage) -> bool {
    msg.language.as_deref() == Some(CONTENT_LANGUAGE)
}

/// Attach a UNL graph as the content (bare list format, UTF-8, `language = "UNL"`).
/// For a message that also carries a data payload, use [`set_message_content`].
pub fn set_unl_content(msg: &mut AclMessage, graph: &UnlGraph) {
    msg.content = unl_parser::serialize_list(graph).into_bytes();
    msg.language = Some(CONTENT_LANGUAGE.to_string());
    msg.encoding = Some("utf-8".to_string());
}

/// Attach a message as a typed-block container: a `UNL ` block (the semantic
/// content) plus a `DATA` block (the payload the UNL describes).
pub fn set_message_content(msg: &mut AclMessage, graph: &UnlGraph, data: &[u8]) {
    let blocks = BlockFile::new()
        .with(TAG_UNL, unl_parser::serialize_list(graph).into_bytes())
        .with(TAG_DATA, data.to_vec());
    msg.content = blocks.encode();
    msg.language = Some(CONTENT_LANGUAGE.to_string());
    msg.encoding = Some("application/x-fipa-blocks".to_string());
}

/// The `DATA` block of a message's content (empty if none / not a block message).
pub fn message_data(msg: &AclMessage) -> Vec<u8> {
    if BlockFile::is_block_container(&msg.content) {
        BlockFile::decode(&msg.content)
            .ok()
            .and_then(|b| b.get(TAG_DATA).map(<[u8]>::to_vec))
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Parse the UNL graph from a message's content — the `UNL ` block of a block
/// container, or the whole content as a bare UNL list (back-compat).
pub fn unl_graph(msg: &AclMessage) -> Result<UnlGraph, UnlError> {
    if !is_unl(msg) {
        return Err(UnlError::NotUnl);
    }
    let unl_owned;
    let unl_bytes: &[u8] = if BlockFile::is_block_container(&msg.content) {
        let blocks = BlockFile::decode(&msg.content).map_err(|e| UnlError::Block(e.to_string()))?;
        unl_owned = blocks.get(TAG_UNL).ok_or(UnlError::NotUnl)?.to_vec();
        &unl_owned
    } else {
        &msg.content
    };
    let text = std::str::from_utf8(unl_bytes).map_err(|_| UnlError::Utf8)?;
    Ok(unl_parser::parse_sentence(text)?)
}

/// A sanitized inbound message — decoded and verified in the node, ready to hand
/// to the agent. Nothing reaches the WASM until it is this.
#[derive(Debug, Clone)]
pub struct Inbound {
    pub performative: i32,
    pub sender: Option<AgentId>,
    pub conversation_id: Option<String>,
    /// The decoded, in-vocabulary semantic content.
    pub graph: UnlGraph,
    /// The data payload the UNL describes.
    pub data: Vec<u8>,
}

/// Sanitize an inbound message in the node: verify against the agent's rules,
/// then decode. `Err` is the `not-understood` reply (reject, never delivered);
/// `Ok(Some)` is the decoded `(UNL, data)` for the agent; `Ok(None)` is a
/// non-UNL message (not this pipeline's concern — deliver by the raw path).
pub fn sanitize_inbound(
    msg: &AclMessage,
    my_name: &str,
    verifier: &dyn ContentVerifier,
) -> Result<Option<Inbound>, AclMessage> {
    crate::flow!("recv: sanitizing message '{}' for agent '{}'", msg.message_id, my_name);
    if verifier.verify(msg).is_err() {
        crate::flow!("recv:   REJECTED — out of vocabulary → not-understood");
        return Err(crate::content::verify::not_understood(msg, my_name));
    }
    if !is_unl(msg) {
        crate::flow!("recv:   non-UNL content → raw path");
        return Ok(None);
    }
    let graph = unl_graph(msg).map_err(|_| crate::content::verify::not_understood(msg, my_name))?;
    let data = message_data(msg);
    crate::flow!(
        "recv:   verified ✓, decoded UNL ({} relation(s)) + {} data byte(s) → deliver to agent",
        graph.relations.len(),
        data.len()
    );
    Ok(Some(Inbound {
        performative: msg.performative,
        sender: msg.sender.clone(),
        conversation_id: msg.conversation_id.clone(),
        graph,
        data,
    }))
}

/// Node-side registry of agents' vocabularies, so the node can check an outgoing
/// message against the **receiver's** words before transmitting — "will the
/// receiver understand this?".
#[derive(Default)]
pub struct VocabRegistry {
    by_agent: HashMap<String, Vocabulary>,
}

impl VocabRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) an agent's vocabulary.
    pub fn register(&mut self, agent: impl Into<String>, vocab: Vocabulary) {
        self.by_agent.insert(agent.into(), vocab);
    }

    pub fn get(&self, agent: &str) -> Option<&Vocabulary> {
        self.by_agent.get(agent)
    }

    pub fn knows(&self, agent: &str) -> bool {
        self.by_agent.contains_key(agent)
    }
}

/// The sender flow (validate + package): check the outgoing UNL against the
/// **receiver's** vocabulary, then package it into a message ready to transmit.
/// `Err` = the receiver would not understand it (diagnostics; do not transmit).
/// An unknown receiver (no vocabulary on file) is packaged optimistically — the
/// node cannot pre-check it.
pub fn package_outbound(
    sender: &str,
    receiver: &str,
    graph: &UnlGraph,
    body: &[u8],
    registry: &VocabRegistry,
) -> Result<AclMessage, Vec<Diagnostic>> {
    crate::flow!("send: '{}' → '{}' : validating UNL against receiver's vocabulary", sender, receiver);
    if let Some(vocab) = registry.get(receiver) {
        if let Err(diags) = unl_validator::verify_vocabulary(graph, vocab) {
            crate::flow!(
                "send:   receiver '{}' would NOT understand ({} issue(s)) → not transmitted",
                receiver,
                diags.len()
            );
            return Err(diags);
        }
    } else {
        crate::flow!("send:   receiver '{}' vocabulary unknown → optimistic send", receiver);
    }
    let mut msg = AclMessage {
        message_id: new_id(),
        performative: Performative::Inform as i32,
        sender: Some(agent_id(sender)),
        receivers: vec![agent_id(receiver)],
        reply_to: None,
        protocol: None,
        conversation_id: None,
        in_reply_to: None,
        reply_with: None,
        reply_by: None,
        language: None,
        encoding: None,
        ontology: None,
        content: Vec::new(),
        user_properties: HashMap::new(),
    };
    set_message_content(&mut msg, graph, body);
    crate::flow!("send:   packaged UNL+DATA message '{}' → transmit", msg.message_id);
    Ok(msg)
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
    use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw};
    use unl_kb::ConceptFeatures;

    /// A small vocabulary that knows cat/animal and the `icl` relation.
    fn vocab() -> Vocabulary {
        let feat = || ConceptFeatures {
            category: LexCategory::Nominal,
            abstract_: false,
            gloss: None,
        };
        let mut v = Vocabulary::new();
        v.allow_concept(2, feat(), vec![], vec![], &["animal"]);
        v.allow_concept(1, feat(), vec![2], vec![], &["cat"]);
        v.allow_relations([RelationTag::Icl]);
        v
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
    fn unl_verifier_accepts_in_vocab_content() {
        let mut m = base_msg(Performative::Inform, "a", "b");
        set_unl_content(&mut m, &cat_icl_animal());
        assert!(UnlVerifier::new(vocab()).verify(&m).is_ok());
    }

    #[test]
    fn unl_verifier_passes_through_non_unl() {
        // No UNL content => not this verifier's concern => deliver.
        let m = base_msg(Performative::Request, "a", "b");
        assert!(UnlVerifier::new(vocab()).verify(&m).is_ok());
    }

    #[test]
    fn unl_verifier_rejects_out_of_vocab_then_not_understood() {
        // "do"/agt are outside the vocabulary (and node 99 dangles).
        let mut bad = base_msg(Performative::Request, "alice", "bob");
        set_unl_content(&mut bad, &dangling());
        assert!(UnlVerifier::new(vocab()).verify(&bad).is_err());

        // The FIPA layer builds the not-understood reply (content-agnostic).
        let nu = crate::content::verify::not_understood(&bad, "bob");
        assert_eq!(nu.performative, Performative::NotUnderstood as i32);
        assert_eq!(nu.sender.unwrap().name, "bob");
        assert_eq!(nu.receivers[0].name, "alice");
        assert_eq!(nu.in_reply_to.as_deref(), Some("r1"));
        assert_eq!(nu.conversation_id.as_deref(), Some("c-1"));
        assert!(nu.language.is_none()); // not-understood carries no content language
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
    fn unl_verifier_from_agent_bundle() {
        use crate::content::block::{BlockFile, TAG_WASM};
        // The agent ships its rules as a UNL block in its bundle (data); the node
        // reads the block and builds the verifier (process).
        let bundle = BlockFile::new()
            .with(TAG_WASM, vec![0, 1, 2])
            .with(TAG_UNL, vocabulary_block(&vocab()));
        let verifier = UnlVerifier::from_bundle(&bundle).expect("has UNL block").unwrap();

        let mut good = base_msg(Performative::Inform, "a", "b");
        set_unl_content(&mut good, &cat_icl_animal());
        assert!(verifier.verify(&good).is_ok());

        let mut bad = base_msg(Performative::Request, "a", "b");
        set_unl_content(&mut bad, &dangling());
        assert!(verifier.verify(&bad).is_err());
    }

    #[test]
    fn no_unl_block_means_no_verifier() {
        use crate::content::block::{BlockFile, TAG_WASM};
        let bundle = BlockFile::new().with(TAG_WASM, vec![0]);
        assert!(UnlVerifier::from_bundle(&bundle).is_none());
    }

    #[test]
    fn message_blocks_unl_plus_data_roundtrip() {
        let mut m = base_msg(Performative::Inform, "a", "b");
        set_message_content(&mut m, &cat_icl_animal(), b"payload");
        assert!(is_unl(&m));
        assert_eq!(first_tag(&m), RelationTag::Icl); // unl_graph reads the UNL block
        assert_eq!(message_data(&m), b"payload");
    }

    #[test]
    fn sanitize_inbound_decodes_valid_rejects_invalid_passes_non_unl() {
        let verifier = UnlVerifier::new(vocab());

        // Valid block message => Some(Inbound) with decoded graph + data.
        let mut good = base_msg(Performative::Inform, "alice", "bob");
        set_message_content(&mut good, &cat_icl_animal(), b"23.4");
        let inbound = sanitize_inbound(&good, "bob", &verifier).unwrap().expect("decoded");
        assert_eq!(inbound.graph.relations[0].tag, RelationTag::Icl);
        assert_eq!(inbound.data, b"23.4");
        assert_eq!(inbound.sender.unwrap().name, "alice");

        // Out-of-vocab => Err(not_understood), never delivered.
        let mut bad = base_msg(Performative::Request, "alice", "bob");
        set_message_content(&mut bad, &dangling(), b"");
        let nu = sanitize_inbound(&bad, "bob", &verifier).unwrap_err();
        assert_eq!(nu.performative, Performative::NotUnderstood as i32);

        // Non-UNL => Ok(None), delivered by the raw path.
        let plain = base_msg(Performative::Request, "alice", "bob");
        assert!(sanitize_inbound(&plain, "bob", &verifier).unwrap().is_none());
    }

    #[test]
    fn sanitize_inbound_back_compat_bare_unl_list() {
        let verifier = UnlVerifier::new(vocab());
        let mut m = base_msg(Performative::Inform, "a", "b");
        set_unl_content(&mut m, &cat_icl_animal()); // bare list, no blocks
        let inbound = sanitize_inbound(&m, "me", &verifier).unwrap().expect("decoded");
        assert_eq!(inbound.graph.relations[0].tag, RelationTag::Icl);
        assert!(inbound.data.is_empty());
    }

    #[test]
    fn unl_verifier_sanitize_decodes_to_unl_and_body() {
        let v = UnlVerifier::new(vocab());

        let mut m = base_msg(Performative::Inform, "a", "b");
        set_message_content(&mut m, &cat_icl_animal(), b"payload");
        let decoded = v.sanitize(&m).unwrap().expect("decoded");
        assert!(!decoded.unl.is_empty());
        assert_eq!(decoded.body, b"payload");

        let mut bad = base_msg(Performative::Request, "a", "b");
        set_message_content(&mut bad, &dangling(), b"");
        assert!(v.sanitize(&bad).is_err()); // out-of-vocab

        let plain = base_msg(Performative::Request, "a", "b");
        assert!(v.sanitize(&plain).unwrap().is_none()); // non-UNL
    }

    #[test]
    fn package_outbound_validates_against_receiver() {
        // Receiver understands => packaged message with UNL + DATA blocks.
        let mut reg = VocabRegistry::new();
        reg.register("bob", vocab());
        let msg = package_outbound("alice", "bob", &cat_icl_animal(), b"hi", &reg).unwrap();
        assert_eq!(msg.sender.as_ref().unwrap().name, "alice");
        assert_eq!(msg.receivers[0].name, "bob");
        assert_eq!(first_tag(&msg), RelationTag::Icl);
        assert_eq!(message_data(&msg), b"hi");

        // Receiver missing the words => rejected, no message.
        let mut narrow = VocabRegistry::new();
        let mut empty_vocab = Vocabulary::new();
        empty_vocab.allow_relations([RelationTag::Icl]); // knows the relation, no concepts
        narrow.register("bob", empty_vocab);
        assert!(package_outbound("alice", "bob", &cat_icl_animal(), b"", &narrow).is_err());

        // Unknown receiver => optimistic packaged message (can't pre-check).
        let empty = VocabRegistry::new();
        assert!(package_outbound("alice", "carol", &cat_icl_animal(), b"", &empty).is_ok());
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
