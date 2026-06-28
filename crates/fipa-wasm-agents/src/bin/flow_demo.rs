// flow_demo.rs - A small node that exercises the UNL message flow end to end.
//
// Two scenarios run through the real node engine (verify against the receiver's
// vocabulary, decode, package, validate-against-receiver-before-send):
//   1. a chat between two agents;
//   2. a buyer that searches for and buys a book.
// Plus a WASM-boundary demo: deliver content into a guest via config(UNL, body)
// and capture the reply the guest emits via send-unl.
//
// Run `cargo run --bin flow-demo --features flow-trace` to see every engine step
// ([flow] lines); without the feature you see just the scenario narration.

use fipa_wasm_agents::content::unl::{self, UnlVerifier, VocabRegistry};
use fipa_wasm_agents::content::verify::ContentVerifier;
use fipa_wasm_agents::proto;
use fipa_wasm_agents::wasm::WasmRuntime;
use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw, UnlGraph};
use unl_kb::{ConceptFeatures, Vocabulary};

fn feat() -> ConceptFeatures {
    ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None }
}

/// A vocabulary that knows the given concept lemmas and the agt/obj relations.
fn vocab(lemmas: &[&str]) -> Vocabulary {
    let mut v = Vocabulary::new();
    for (i, lemma) in lemmas.iter().enumerate() {
        v.allow_concept(100 + i as u64, feat(), vec![], vec![], &[lemma]);
    }
    v.allow_relations([RelationTag::Agt, RelationTag::Obj]);
    v
}

/// A graph `head` with `agt -> actor` and `obj -> object` (a simple predication).
fn act(head: &str, actor: &str, object: &str) -> UnlGraph {
    let mut g = UnlGraph::new();
    g.insert_node("00", Uw::new(Uci::ucn(head)));
    g.insert_node("01", Uw::new(Uci::ucn(actor)));
    g.insert_node("02", Uw::new(Uci::ucn(object)));
    g.entry = Some("00".into());
    g.add_relation(Relation::between(RelationTag::Agt, "00".into(), "01".into()));
    g.add_relation(Relation::between(RelationTag::Obj, "00".into(), "02".into()));
    g
}

/// One hop: `from` sends `graph`+`body` to `to`. The node validates the message
/// against the receiver before sending, then the receiver's node sanitizes it.
fn deliver(
    from: &str,
    to: &str,
    graph: &UnlGraph,
    body: &[u8],
    reg: &VocabRegistry,
    to_verifier: &dyn ContentVerifier,
) -> bool {
    let msg = match unl::package_outbound(from, to, graph, body, reg) {
        Ok(m) => m,
        Err(d) => {
            println!("  ✗ {from} → {to}: receiver would not understand ({} issue(s)); not sent", d.len());
            return false;
        }
    };
    match unl::sanitize_inbound(&msg, to, to_verifier) {
        Ok(Some(inb)) => {
            println!(
                "  ✓ {from} → {to}: understood ({} relations, {} data bytes)",
                inb.graph.relations.len(),
                inb.data.len()
            );
            true
        }
        Ok(None) => {
            println!("  · {from} → {to}: non-UNL content");
            false
        }
        Err(_) => {
            println!("  ✗ {from} → {to}: not-understood");
            false
        }
    }
}

fn scenario_chat() {
    println!("\n── Scenario 1: chat ──────────────────────────────────");
    let alice = vocab(&["greet", "alice", "bob"]);
    let bob = vocab(&["greet", "alice", "bob"]);
    let mut reg = VocabRegistry::new();
    reg.register("alice", alice.clone());
    reg.register("bob", bob.clone());
    let v_alice = UnlVerifier::new(alice);
    let v_bob = UnlVerifier::new(bob);

    deliver("alice", "bob", &act("greet", "alice", "bob"), b"hi there!", &reg, &v_bob);
    deliver("bob", "alice", &act("greet", "bob", "alice"), b"hello :)", &reg, &v_alice);
}

fn scenario_book() {
    println!("\n── Scenario 2: search & buy a book ───────────────────");
    let words = ["search", "offer", "buy", "buyer", "store", "book"];
    let buyer = vocab(&words);
    let store = vocab(&words); // the store does NOT know "refund"
    let mut reg = VocabRegistry::new();
    reg.register("buyer", buyer.clone());
    reg.register("store", store.clone());
    let v_buyer = UnlVerifier::new(buyer);
    let v_store = UnlVerifier::new(store);

    deliver("buyer", "store", &act("search", "buyer", "book"), b"title=Dune", &reg, &v_store);
    deliver("store", "buyer", &act("offer", "store", "book"), b"price=9.99", &reg, &v_buyer);
    deliver("buyer", "store", &act("buy", "buyer", "book"), b"title=Dune", &reg, &v_store);

    println!("  -- buyer asks for a 'refund' (a word the store has no vocabulary for):");
    deliver("buyer", "store", &act("refund", "buyer", "book"), b"", &reg, &v_store);
}

// A tiny WASM agent: on config() it emits a canned reply via send-unl. It needs
// memory + alloc (so the host can write the inbound bytes) + init + config.
const ECHO_AGENT: &str = r#"
(module
  (import "fipa:agent/messaging" "send-unl"
    (func $send (param i32 i32 i32 i32 i32 i32)))
  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 1024))
  (data (i32.const 0) "alice")
  (data (i32.const 16) "agt(greet,alice)")
  (data (i32.const 64) "hi from wasm")
  (func (export "init"))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $n)))
    (local.get $p))
  (func (export "config") (param i32 i32 i32 i32)
    (call $send
      (i32.const 0) (i32.const 5)    ;; receiver "alice"
      (i32.const 16) (i32.const 16)  ;; unl "agt(greet,alice)"
      (i32.const 64) (i32.const 12)))) ;; body "hi from wasm"
"#;

fn scenario_wasm_boundary() {
    println!("\n── Scenario 3: WASM agent boundary (config → agent → send-unl) ──");
    let caps = proto::AgentCapabilities { max_execution_time_ms: 1000, ..Default::default() };
    let mut rt = match WasmRuntime::new(ECHO_AGENT.as_bytes(), &caps) {
        Ok(rt) => rt,
        Err(e) => {
            println!("  (skipped: {e})");
            return;
        }
    };
    rt.call_init().expect("init");
    // Deliver a decoded message into the agent; it replies during config().
    rt.call_config(b"agt(greet,bob)", b"hello agent").expect("config");
    for intent in rt.take_unl_sends() {
        println!(
            "  ✓ agent replied → {}: unl=\"{}\", body=\"{}\"",
            intent.receiver,
            String::from_utf8_lossy(&intent.unl),
            String::from_utf8_lossy(&intent.body),
        );
    }
}

fn main() {
    println!("=== UNL message-flow demo ===");
    println!("(run with `--features flow-trace` to see each engine step as [flow] lines)");
    scenario_chat();
    scenario_book();
    scenario_wasm_boundary();
    println!("\nDone.");
}
