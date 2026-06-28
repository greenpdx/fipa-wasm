// ping_pong.rs - Two instances of the same WASM chat agent volley a message
// back and forth over the actor runtime.
//
// The agent (agents/chat_agent.wat) is generic: it learns its PEER's id from
// its DATA block (delivered as the seed body). We instantiate it twice and pick
// each one's peer — ping ↔ pong — then kick it off. Each bounces the ball back
// to its peer up to a bounded number of volleys, then stops.
//
// Run `cargo run --bin ping-pong --features flow-trace` to watch the volleys.

use actix::prelude::*;
use fipa_wasm_agents::content::block::{BlockFile, TAG_DATA, TAG_UNL, TAG_WASM};
use fipa_wasm_agents::content::unl::{set_message_content, vocabulary_block};
use fipa_wasm_agents::proto;
use fipa_wasm_agents::{AgentConfig, DeliverMessage, RestartStrategy, SpawnAgent, Supervisor};
use std::time::Duration;
use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw, UnlGraph};
use unl_kb::{ConceptFeatures, Vocabulary};

const CHAT_AGENT: &str = include_str!("../../agents/chat_agent.wat");

fn vocab(words: &[&str]) -> Vocabulary {
    let mut v = Vocabulary::new();
    for (i, w) in words.iter().enumerate() {
        let feat = ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None };
        v.allow_concept(100 + i as u64, feat, vec![], vec![], &[w]);
    }
    v.allow_relations([RelationTag::Agt]);
    v
}

/// The agent bundle: the (shared) chat-agent WASM, the vocabulary, and the
/// DATA block carrying the PEER's id — this is where we pick the other chat.
fn bundle(v: &Vocabulary, peer: &str) -> Vec<u8> {
    BlockFile::new()
        .with(TAG_WASM, CHAT_AGENT.as_bytes().to_vec())
        .with(TAG_UNL, vocabulary_block(v))
        .with(TAG_DATA, peer.as_bytes().to_vec())
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

/// The ball: `agt(ping, pong)`.
fn ball() -> UnlGraph {
    let mut g = UnlGraph::new();
    g.insert_node("00", Uw::new(Uci::ucn("ping")));
    g.insert_node("01", Uw::new(Uci::ucn("pong")));
    g.entry = Some("00".into());
    g.add_relation(Relation::between(RelationTag::Agt, "00".into(), "01".into()));
    g
}

fn kickoff(to: &str) -> proto::AclMessage {
    let mut m = proto::AclMessage {
        message_id: "serve".to_string(),
        performative: proto::Performative::Inform as i32,
        sender: Some(agent_id("starter")),
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
    set_message_content(&mut m, &ball(), b"ball");
    m
}

async fn run() {
    let v = vocab(&["ping", "pong"]);

    let sup = Supervisor::new("chat-node".to_string()).start();
    // Pick each agent's peer: ping talks to pong, pong talks to ping.
    let _ = sup.send(SpawnAgent { config: agent_config("ping", bundle(&v, "pong")) }).await;
    let _ = sup.send(SpawnAgent { config: agent_config("pong", bundle(&v, "ping")) }).await;
    tokio::time::sleep(Duration::from_millis(60)).await; // start + seed peer ids

    println!("\n────── ping-pong: serve to 'ping' ──────");
    sup.do_send(DeliverMessage { message: kickoff("ping") });
    tokio::time::sleep(Duration::from_millis(400)).await;
}

fn main() {
    println!("=== UNL ping-pong: two WASM chat agents ===");
    println!("(run with `--features flow-trace` to see each volley)");
    actix::System::new().block_on(run());
    println!("\nDone.");
}
