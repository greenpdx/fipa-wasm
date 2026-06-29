// book_demo.rs — the full book-buy conversation over the Router, with the buyer
// (BA) running as a real wasm32 agent (ba-agent), then deliberately broken to
// locate problems.
//
//   cargo build -p ba-agent --target wasm32-unknown-unknown   # build BA wasm
//   cargo run --bin book-demo --features flow-trace           # run it
//
// If the BA wasm isn't built, the demo falls back to running the same Buyer
// natively (ba-agent is cdylib+rlib — one source, both targets).

use fipa_wasm_agents::identity::{AgentId, Header};
use fipa_wasm_agents::process::Router;
use fipa_wasm_agents::proto;
use fipa_wasm_agents::wasm::{AgentRuntime, NativeRuntime, WasmRuntime};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;
use uuid::Uuid;

// ── message helpers (for BS + reading BA's verdict) ─────────────────────

fn verb_subject(unl: &str) -> Option<(String, String)> {
    let g = parse_sentence(unl).ok()?;
    let rel = g.relations.first()?;
    Some((word(&rel.source)?, word(&rel.target)?))
}
fn word(n: &NodeRef) -> Option<String> {
    if let NodeRef::Inline(uw) = n {
        if let Uci::Ucn { root, .. } = &uw.uci {
            return Some(root.to_string());
        }
    }
    None
}
fn jstatus(body: &[u8]) -> String {
    jfield(body, "status").unwrap_or_default()
}
fn jfield(body: &[u8], key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

// ── BS: the seller (native) ─────────────────────────────────────────────

struct Seller {
    has_ltg: bool,
    buyers: BTreeMap<String, String>, // order -> buyer
}
impl Seller {
    fn new(has_ltg: bool) -> Self {
        Seller { has_ltg, buyers: BTreeMap::new() }
    }
}
impl Agent for Seller {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        let Some((verb, subject)) = verb_subject(unl) else { return };
        match verb.as_str() {
            "catalog" => {
                let books = if self.has_ltg {
                    serde_json::json!([{"title":"LtG","price":999},{"title":"Other","price":500}])
                } else {
                    serde_json::json!([{"title":"Other","price":500}])
                };
                let from = ctx.from().to_string();
                ctx.send(from, "obj(catalog, systemdynamics)", serde_json::to_vec(&books).unwrap());
            }
            "receipt" => match jstatus(body).as_str() {
                "held" => {
                    if let Some(buyer) = jfield(body, "buyer") {
                        self.buyers.insert(subject.clone(), buyer);
                        ctx.send("pa", format!("obj(accept, {subject})"), vec![]);
                    }
                }
                "paid" => {
                    if let Some(buyer) = self.buyers.get(&subject) {
                        ctx.send(buyer.clone(), format!("obj(deliver, {subject})"), vec![]);
                    }
                }
                "cancelled" => {
                    self.buyers.remove(&subject);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

// ── BA: load the wasm agent (fallback to native) ────────────────────────

fn ba_wasm_path() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/wasm32-unknown-unknown/debug/ba_agent.wasm")
}

fn ba_is_wasm() -> bool {
    std::fs::metadata(ba_wasm_path()).is_ok()
}

fn ba_runtime() -> Box<dyn AgentRuntime> {
    if let Ok(bytes) = std::fs::read(ba_wasm_path()) {
        let caps = proto::AgentCapabilities { max_execution_time_ms: 1000, ..Default::default() };
        if let Ok(rt) = WasmRuntime::new(&bytes, &caps) {
            return Box::new(rt);
        }
    }
    Box::new(NativeRuntime::new(ba_agent::Buyer::new()))
}

// ── wiring + scenarios ──────────────────────────────────────────────────

fn temp_path() -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("book-pa-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

struct Scenario {
    ba_funds: u64,
    df_has_provider: bool,
    ams_has_address: bool,
    bs_has_book: bool,
}

/// Mint a UUID identity with a friendly name (infra would persist in production).
fn aid(name: &str) -> AgentId {
    let header = Header { type_id: Uuid::new_v4(), desc: format!("{name} service"), name: Some(name.into()) };
    AgentId::spawn(&header)
}

fn run(s: &Scenario) -> String {
    // Every agent is a UUID; df/ams/pa keep well-known aliases (bootstrap), the
    // seller's UUID is discovered via DF, BA's via the kickoff.
    let df = aid("df");
    let ams = aid("ams");
    let pa = aid("pa");
    let seller = aid("bookSeller");
    let ba = aid("BA");

    let mut df_agent = df_agent::Df::new();
    if s.df_has_provider {
        let seed = serde_json::json!({ "bookselling": [seller.id()] });
        df_agent.on_seed(seed.to_string().as_bytes(), &mut Ctx::new());
    }
    let mut ams_agent = ams_agent::Ams::new();
    if s.ams_has_address {
        let seed = serde_json::json!({ "records": { seller.id(): "127.0.0.1:9001" } });
        ams_agent.on_seed(seed.to_string().as_bytes(), &mut Ctx::new());
    }
    let mut pa_agent = pa_agent::Pa::open(temp_path()).unwrap();
    pa_agent.credit(ba.id(), s.ba_funds);

    let mut r = Router::new();
    for a in [&df, &ams, &pa, &seller, &ba] {
        r.bind_alias(a.name.clone().unwrap(), a.id()); // readable traces + bootstrap
    }
    r.add(df.id(), Box::new(NativeRuntime::new(df_agent)));
    r.add(ams.id(), Box::new(NativeRuntime::new(ams_agent)));
    r.add(pa.id(), Box::new(NativeRuntime::new(pa_agent)));
    r.add(seller.id(), Box::new(NativeRuntime::new(Seller::new(s.bs_has_book))));
    r.add(ba.id(), ba_runtime()); // ← the buyer, as wasm (or native fallback)

    r.send("boot", &ba.id(), b"obj(start, buy)", b"");
    r.run(200);

    match r.outbox.iter().find(|e| e.to == "result") {
        Some(e) => {
            let verb = verb_subject(&String::from_utf8_lossy(&e.unl)).map(|(v, _)| v).unwrap_or_default();
            if verb == "bought" {
                "✓ bought LtG".to_string()
            } else {
                format!("✗ failed: {}", String::from_utf8_lossy(&e.body))
            }
        }
        None => "✗ stalled (no verdict — conversation got stuck)".to_string(),
    }
}

fn main() {
    println!("=== book-buy conversation (run with --features flow-trace to see each hop) ===");
    println!(
        "BA running as: {}",
        if ba_is_wasm() {
            "wasm32 (real mobile agent)"
        } else {
            "native fallback — build it: cargo build -p ba-agent --target wasm32-unknown-unknown"
        }
    );

    let scenarios = [
        ("happy path", Scenario { ba_funds: 10000, df_has_provider: true, ams_has_address: true, bs_has_book: true }),
        ("broken: buyer underfunded", Scenario { ba_funds: 100, df_has_provider: true, ams_has_address: true, bs_has_book: true }),
        ("broken: no seller for the service", Scenario { ba_funds: 10000, df_has_provider: false, ams_has_address: true, bs_has_book: true }),
        ("broken: seller has no address", Scenario { ba_funds: 10000, df_has_provider: true, ams_has_address: false, bs_has_book: true }),
        ("broken: seller lacks the book", Scenario { ba_funds: 10000, df_has_provider: true, ams_has_address: true, bs_has_book: false }),
    ];

    for (name, s) in scenarios {
        println!("\n── {name} ──");
        println!("   result: {}", run(&s));
    }
    println!();
}
