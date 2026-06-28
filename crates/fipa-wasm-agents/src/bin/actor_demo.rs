// actor_demo.rs - The full actor runtime: a Supervisor spawns real WASM agents
// (as typed-block bundles) that message each other through the actor system.
//
// Each agent is a bundle of a WASM block (a tiny "responder" guest) + a UNL
// block (its vocabulary). The node reads the UNL block to build the agent's
// verifier; the agent's WASM is driven through init()/config(UNL, body), emits
// replies via send-unl, and the node validates + packages + routes them.
//
// Two conversations run over the runtime:
//   1. chat  — alice greets bob, bob greets back, alice acknowledges;
//   2. book  — buyer searches the store, store offers, buyer buys.
//
// Run `cargo run --bin actor-demo --features flow-trace` to watch every engine
// step ([flow] lines) across the real receive → config → send-unl → package →
// route loop.

use actix::prelude::*;
use fipa_wasm_agents::content::block::{BlockFile, TAG_UNL, TAG_WASM};
use fipa_wasm_agents::content::unl::{set_message_content, vocabulary_block};
use fipa_wasm_agents::proto;
use fipa_wasm_agents::{AgentConfig, DeliverMessage, RestartStrategy, SpawnAgent, Supervisor};
use std::time::Duration;
use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw, UnlGraph};
use unl_kb::{ConceptFeatures, Vocabulary};

/// A tiny WASM agent: on the first *message* config (not the vocabulary seed,
/// whose UNL begins with '{'), it emits one canned reply via `send-unl`.
fn responder(reply_to: &str, reply_unl: &str, reply_body: &str) -> String {
    format!(
        r#"(module
  (import "fipa:agent/messaging" "send-unl" (func $send (param i32 i32 i32 i32 i32 i32)))
  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 4096))
  (global $replied (mut i32) (i32.const 0))
  (data (i32.const 0) "{to}")
  (data (i32.const 256) "{unl}")
  (data (i32.const 512) "{body}")
  (func (export "init"))
  (func (export "run") (result i32) (i32.const 1))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $n)))
    (local.get $p))
  (func (export "config") (param $up i32) (param $ul i32) (param $bp i32) (param $bl i32)
    (if (i32.and
          (i32.eqz (global.get $replied))
          (i32.ne (i32.load8_u (local.get $up)) (i32.const 0x7b)))
      (then
        (global.set $replied (i32.const 1))
        (call $send
          (i32.const 0) (i32.const {to_len})
          (i32.const 256) (i32.const {unl_len})
          (i32.const 512) (i32.const {body_len}))))))"#,
        to = reply_to,
        unl = reply_unl,
        body = reply_body,
        to_len = reply_to.len(),
        unl_len = reply_unl.len(),
        body_len = reply_body.len(),
    )
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

/// The agent's deployable bundle: a WASM block + a UNL (vocabulary) block.
fn bundle(wat: &str, v: &Vocabulary) -> Vec<u8> {
    BlockFile::new()
        .with(TAG_WASM, wat.as_bytes().to_vec())
        .with(TAG_UNL, vocabulary_block(v))
        .encode()
}

fn agent_id(name: &str) -> proto::AgentId {
    proto::AgentId { name: name.to_string(), addresses: vec![], resolvers: vec![] }
}

fn agent_config(name: &str, bundle: Vec<u8>) -> AgentConfig {
    AgentConfig {
        id: agent_id(name),
        wasm_module: bundle,
        capabilities: proto::AgentCapabilities {
            max_memory_bytes: 16 * 1024 * 1024,
            max_execution_time_ms: 1000,
            storage_quota_bytes: 1024 * 1024,
            ..Default::default()
        },
        initial_state: None,
        restart_strategy: RestartStrategy::default(),
    }
}

/// A `head agt arg` predication, e.g. `agt(greet, alice)`.
fn pred(head: &str, arg: &str) -> UnlGraph {
    let mut g = UnlGraph::new();
    g.insert_node("00", Uw::new(Uci::ucn(head)));
    g.insert_node("01", Uw::new(Uci::ucn(arg)));
    g.entry = Some("00".into());
    g.add_relation(Relation::between(RelationTag::Agt, "00".into(), "01".into()));
    g
}

fn unl_message(from: &str, to: &str, graph: &UnlGraph, body: &[u8]) -> proto::AclMessage {
    let mut m = proto::AclMessage {
        message_id: format!("kick-{from}-{to}"),
        performative: proto::Performative::Inform as i32,
        sender: Some(agent_id(from)),
        receivers: vec![agent_id(to)],
        reply_to: None,
        protocol: None,
        conversation_id: None,
        in_reply_to: None,
        reply_with: None,
        reply_by: None,
        language: None,
        encoding: None,
        ontology: None,
        content: vec![],
        user_properties: Default::default(),
    };
    set_message_content(&mut m, graph, body);
    m
}

async fn run() {
    let chat = vocab(&["greet", "alice", "bob"]);
    let book = vocab(&["search", "offer", "buy", "book"]);

    // Each agent: a responder guest (its canned reply) + its vocabulary.
    let agents: Vec<(&str, Vec<u8>)> = vec![
        ("alice", bundle(&responder("bob", "agt(greet, alice)", "thanks!"), &chat)),
        ("bob", bundle(&responder("alice", "agt(greet, bob)", "hello!"), &chat)),
        ("buyer", bundle(&responder("store", "agt(buy, book)", "title=Dune"), &book)),
        ("store", bundle(&responder("buyer", "agt(offer, book)", "price=9.99"), &book)),
    ];

    let sup = Supervisor::new("demo-node".to_string()).start();
    for (name, b) in agents {
        let _ = sup.send(SpawnAgent { config: agent_config(name, b) }).await;
    }
    tokio::time::sleep(Duration::from_millis(60)).await; // let them start + seed

    println!("\n────── chat: alice → bob ──────");
    sup.do_send(DeliverMessage { message: unl_message("alice", "bob", &pred("greet", "alice"), b"hi there") });
    tokio::time::sleep(Duration::from_millis(250)).await;

    println!("\n────── book: buyer → store ──────");
    sup.do_send(DeliverMessage { message: unl_message("buyer", "store", &pred("search", "book"), b"title=Dune") });
    tokio::time::sleep(Duration::from_millis(250)).await;
}

fn main() {
    println!("=== UNL actor-runtime demo ===");
    println!("(run with `--features flow-trace` to see each engine step as [flow] lines)");
    let system = actix::System::new();
    system.block_on(run());
    println!("\nDone.");
}
