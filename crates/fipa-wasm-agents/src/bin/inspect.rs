// inspect.rs - Walk ONE message through every pipeline stage and print the
// actual struct/data at each step: the wire packet, the FIPA envelope, the UNL
// blocks, validation, the decoded graph, the (UNL, body) handed to WASM, what
// the WASM emits, validation against the receiver, the outgoing FIPA header, and
// the outgoing wire bytes.
//
//   cargo run --bin inspect

use fipa_wasm_agents::content::block::BlockFile;
use fipa_wasm_agents::content::unl::{
    self, set_message_content, vocabulary_block, UnlPackager, UnlVerifier, VocabRegistry,
};
use fipa_wasm_agents::content::verify::{ContentVerifier, OutboundPackager};
use fipa_wasm_agents::proto;
use fipa_wasm_agents::wasm::WasmRuntime;
use std::sync::{Arc, RwLock};
use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw, UnlGraph};
use unl_kb::{ConceptFeatures, Vocabulary};

const CHAT_AGENT: &str = include_str!("../../agents/chat_agent.wat");

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn vocab(words: &[&str]) -> Vocabulary {
    let mut v = Vocabulary::new();
    for (i, w) in words.iter().enumerate() {
        let feat = ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None };
        v.allow_concept(100 + i as u64, feat, vec![], vec![], &[w]);
    }
    v.allow_relations([RelationTag::Agt]);
    v
}

fn agent_id(name: &str) -> proto::AgentId {
    proto::AgentId { name: name.to_string(), addresses: vec![], resolvers: vec![] }
}

/// `agt(greet, alice)` with declared nodes.
fn greet() -> UnlGraph {
    let mut g = UnlGraph::new();
    g.insert_node("00", Uw::new(Uci::ucn("greet")));
    g.insert_node("01", Uw::new(Uci::ucn("alice")));
    g.entry = Some("00".into());
    g.add_relation(Relation::between(RelationTag::Agt, "00".into(), "01".into()));
    g
}

fn rule(s: &str) {
    println!("\n\x1b[1m══ {s} ══\x1b[0m");
}

fn main() {
    // bob's "UNL rules": the vocabulary the node uses to verify/decode for bob.
    let bob_rules = vocab(&["greet", "alice", "bob"]);

    // ─────────────────────────────  INBOUND  ─────────────────────────────
    let mut packet = proto::AclMessage {
        message_id: "m-1".into(),
        performative: proto::Performative::Inform as i32,
        sender: Some(agent_id("alice")),
        receivers: vec![agent_id("bob")],
        reply_to: None,
        protocol: None,
        conversation_id: Some("c-1".into()),
        in_reply_to: None,
        reply_with: None,
        reply_by: None,
        language: None,
        encoding: None,
        ontology: None,
        content: vec![],
        user_properties: Default::default(),
    };
    set_message_content(&mut packet, &greet(), b"hi");

    rule("STAGE 1 · PACKET (incoming wire message: proto::AclMessage)");
    println!("  message_id : {:?}", packet.message_id);
    println!("  content    : {} bytes (typed-block container)", packet.content.len());
    println!("  bytes[..24]: {:02x?}", &packet.content[..packet.content.len().min(24)]);

    rule("STAGE 2 · FIPA DECODE (envelope fields)");
    println!("  performative   : {:?}", proto::Performative::try_from(packet.performative).unwrap());
    println!("  sender         : {}", packet.sender.as_ref().unwrap().name);
    println!("  receivers      : {:?}", packet.receivers.iter().map(|a| &a.name).collect::<Vec<_>>());
    println!("  conversation_id: {:?}", packet.conversation_id);
    println!("  language       : {:?}", packet.language);
    println!("  encoding       : {:?}", packet.encoding);

    rule("STAGE 3 · UNL (content blocks pulled from the container)");
    let blocks = BlockFile::decode(&packet.content).unwrap();
    for b in &blocks.blocks {
        println!("  [{}] {:?}", text(&b.tag), text(&b.data));
    }

    rule("STAGE 4 · VALIDATE (graph vs bob's vocabulary rules)");
    let graph = unl::unl_graph(&packet).unwrap();
    match unl_validator::verify_vocabulary(&graph, &bob_rules) {
        Ok(()) => println!("  ✓ every concept/relation in-vocabulary, structurally sound"),
        Err(d) => println!("  ✗ {} issue(s): {:#?}", d.len(), d),
    }

    rule("STAGE 5 · UNL DECODE (the semantic graph, decoded via bob's rules)");
    println!("{graph:#?}");

    rule("STAGE 6 · INTO WASM (Decoded → config(unl, body))");
    let verifier = UnlVerifier::new(bob_rules.clone());
    let decoded = verifier.sanitize(&packet).unwrap().expect("decoded");
    println!("  unl : {:?}", text(&decoded.unl));
    println!("  body: {:?}", text(&decoded.body));

    // ──────────────────────────────  WASM  ──────────────────────────────
    let caps = proto::AgentCapabilities { max_execution_time_ms: 1000, ..Default::default() };
    let mut rt = WasmRuntime::new(CHAT_AGENT.as_bytes(), &caps).unwrap();
    rt.call_init().unwrap();
    rt.call_config(&vocabulary_block(&bob_rules), b"alice").unwrap(); // seed: vocab + peer id
    rt.call_config(&decoded.unl, &decoded.body).unwrap(); // the message
    let out = rt.take_unl_sends().pop().expect("agent emitted a reply");

    rule("STAGE 7 · OUT OF WASM (OutboundIntent the agent emitted)");
    println!("  receiver: {:?}", out.receiver);
    println!("  unl     : {:?}", text(&out.unl));
    println!("  body    : {:?}", text(&out.body));

    // ────────────────────────────  OUTBOUND  ────────────────────────────
    let mut reg = VocabRegistry::new();
    reg.register("alice", vocab(&["greet", "alice", "bob"])); // the receiver's rules
    let packager = UnlPackager::new(Arc::new(RwLock::new(reg)));

    rule("STAGE 8 · UNL VALIDATE (outgoing UNL vs receiver 'alice')");
    let outgoing = match packager.package("bob", &out.receiver, &out.unl, &out.body) {
        Ok(m) => {
            println!("  ✓ receiver understands → packaged");
            m
        }
        Err(e) => {
            println!("  ✗ {e}");
            return;
        }
    };

    rule("STAGE 9 · FIPA HEADER (outgoing envelope)");
    println!("  performative: {:?}", proto::Performative::try_from(outgoing.performative).unwrap());
    println!("  sender      : {}", outgoing.sender.as_ref().unwrap().name);
    println!("  receivers   : {:?}", outgoing.receivers.iter().map(|a| &a.name).collect::<Vec<_>>());
    println!("  language    : {:?}", outgoing.language);
    println!("  encoding    : {:?}", outgoing.encoding);

    rule("STAGE 10 · OUT (outgoing wire bytes)");
    println!("  content    : {} bytes (typed-block container)", outgoing.content.len());
    let ob = BlockFile::decode(&outgoing.content).unwrap();
    for b in &ob.blocks {
        println!("  [{}] {:?}", text(&b.tag), text(&b.data));
    }
}
