// book_demo.rs — the full book-buy conversation over the Router, with
// authenticated `from`, then deliberately broken to locate problems.
//
//   cargo run --bin book-demo --features flow-trace
//
// Flow (all messages stamped with the authenticated sender by the Router):
//   BA → DF  seek bookselling     DF → BA  provide [bookSeller]
//   BA → AMS locate bookSeller    AMS → BA at {address}
//   BA → BS  catalog              BS → BA  catalog [{LtG, 999}, …]
//   BA → PA  reserve LtG {seller, 999}
//        PA → BA receipt held     PA → BS receipt held {buyer:BA}
//   BS → PA  accept LtG           PA → BA,BS receipt paid   (funds released)
//   BS → BA  deliver LtG          BA → result bought
//
// BA emits its final verdict to "result" (a non-agent) so it lands in the
// router outbox where we can read it.

use fipa_wasm_agents::process::Router;
use fipa_wasm_agents::wasm::NativeRuntime;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

// ── message helpers ─────────────────────────────────────────────────────

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

// ── BA: the buyer (a small conversation state machine) ──────────────────

#[derive(Clone, Copy, PartialEq)]
enum St {
    Init,
    Provider,
    Address,
    Catalog,
    Held,
    Delivery,
    Done,
    Failed,
}

#[derive(Deserialize)]
struct Book {
    title: String,
    price: u64,
}

struct Buyer {
    st: St,
    seller: String,
}
impl Buyer {
    fn new() -> Self {
        Buyer { st: St::Init, seller: String::new() }
    }
}
impl Agent for Buyer {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        let Some((verb, _subj)) = verb_subject(unl) else { return };
        // any deny aborts the purchase.
        if verb == "deny" {
            let reason = jfield(body, "reason").unwrap_or_else(|| "denied".into());
            ctx.send("result", "obj(failed, x)", reason.into_bytes());
            self.st = St::Failed;
            return;
        }
        match (self.st, verb.as_str()) {
            (St::Init, "start") => {
                ctx.send("df", "obj(seek, bookselling)", vec![]);
                self.st = St::Provider;
            }
            (St::Provider, "provide") => {
                let providers: Vec<String> = serde_json::from_slice(body).unwrap_or_default();
                match providers.first() {
                    Some(p) => {
                        self.seller = p.clone();
                        ctx.send("ams", format!("obj(locate, {p})"), vec![]);
                        self.st = St::Address;
                    }
                    None => self.fail(ctx, "no-provider"),
                }
            }
            (St::Address, "at") => match jfield(body, "address") {
                Some(_addr) => {
                    ctx.send(&self.seller, "obj(catalog, systemdynamics)", vec![]);
                    self.st = St::Catalog;
                }
                None => self.fail(ctx, "no-address"),
            },
            (St::Catalog, "catalog") => {
                let books: Vec<Book> = serde_json::from_slice(body).unwrap_or_default();
                match books.iter().find(|b| b.title == "LtG") {
                    Some(b) => {
                        let terms = serde_json::json!({ "seller": self.seller, "amount": b.price });
                        ctx.send("pa", "obj(reserve, LtG)", serde_json::to_vec(&terms).unwrap());
                        self.st = St::Held;
                    }
                    None => self.fail(ctx, "book-not-found"),
                }
            }
            (St::Held, "receipt") => {
                if jstatus(body) == "held" {
                    self.st = St::Delivery; // funds secured; await the book
                }
            }
            (St::Delivery, "deliver") => {
                ctx.send("result", "obj(bought, LtG)", vec![]);
                self.st = St::Done;
            }
            _ => {}
        }
    }
}
impl Buyer {
    fn fail(&mut self, ctx: &mut Ctx, why: &str) {
        ctx.send("result", "obj(failed, x)", why.as_bytes().to_vec());
        self.st = St::Failed;
    }
}

// ── BS: the seller ──────────────────────────────────────────────────────

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
                        self.buyers.insert(subject.clone(), buyer); // reserve the book for the buyer
                        ctx.send("pa", format!("obj(accept, {subject})"), vec![]);
                    }
                }
                "paid" => {
                    if let Some(buyer) = self.buyers.get(&subject) {
                        ctx.send(buyer.clone(), format!("obj(deliver, {subject})"), vec![]); // ship
                    }
                }
                "cancelled" => {
                    self.buyers.remove(&subject); // release the book
                }
                _ => {}
            },
            _ => {}
        }
    }
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

fn run(s: &Scenario) -> String {
    let mut df = df_agent::Df::new();
    if s.df_has_provider {
        df.on_seed(br#"{"bookselling":["bookSeller"]}"#, &mut Ctx::new());
    }
    let mut ams = ams_agent::Ams::new();
    if s.ams_has_address {
        ams.on_seed(br#"{"records":{"bookSeller":"127.0.0.1:9001"}}"#, &mut Ctx::new());
    }
    let mut pa = pa_agent::Pa::open(temp_path()).unwrap();
    pa.credit("BA", s.ba_funds);

    let mut r = Router::new();
    r.add("df", Box::new(NativeRuntime::new(df)));
    r.add("ams", Box::new(NativeRuntime::new(ams)));
    r.add("pa", Box::new(NativeRuntime::new(pa)));
    r.add("bookSeller", Box::new(NativeRuntime::new(Seller::new(s.bs_has_book))));
    r.add("BA", Box::new(NativeRuntime::new(Buyer::new())));

    r.send("boot", "BA", b"obj(start, buy)", b""); // kick off the buyer
    r.run(200);

    // BA's verdict landed in the outbox (addressed to the non-agent "result").
    match r.outbox.iter().find(|e| e.to == "result") {
        Some(e) => {
            let (verb, _) = verb_subject(&String::from_utf8_lossy(&e.unl)).unwrap_or_default_pair();
            if verb == "bought" {
                "✓ bought LtG".to_string()
            } else {
                format!("✗ failed: {}", String::from_utf8_lossy(&e.body))
            }
        }
        None => "✗ stalled (no verdict — conversation got stuck)".to_string(),
    }
}

trait UnwrapPair {
    fn unwrap_or_default_pair(self) -> (String, String);
}
impl UnwrapPair for Option<(String, String)> {
    fn unwrap_or_default_pair(self) -> (String, String) {
        self.unwrap_or_default()
    }
}

fn main() {
    println!("=== book-buy conversation (run with --features flow-trace to see each hop) ===");

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
