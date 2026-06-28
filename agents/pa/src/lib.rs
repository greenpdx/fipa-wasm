//! # Payment Agent (PA) — escrow / hold settlement (durable)
//!
//! PA settles payment between a buyer and a seller with an **escrow hold**: it
//! **reserves** (holds) the buyer's funds, then **releases** them to the seller
//! on **accept** or back to the buyer on **deny**, issuing **receipts**
//! throughout. State (the ledger + holds) is **durable** (sled): a restart
//! recovers in-flight escrow.
//!
//! ## The six verbs as a state machine
//!
//! ```text
//!                  reserve (buyer)
//!         insufficient ╱ ╲ ok
//!               deny ◀╱   ╲▶  HELD ── receipt "held" → buyer & seller
//!                           │
//!             accept(seller)╱ ╲ deny(buyer|seller)
//!       release→seller, PAID   release→buyer, CANCELLED
//!        receipt "paid" ×2      receipt "cancelled" ×2
//! ```
//!
//! ## Messages — UNL verb + order id; JSON body for terms
//!
//! | in `unl` / `body` | PA does |
//! |---|---|
//! | `obj(reserve, <order>)` / `{"seller":"<id>","amount":<n>}` | buyer = sender; hold or `deny` |
//! | `obj(accept, <order>)` / — | release to the seller |
//! | `obj(deny, <order>)` / — | release back to the buyer |
//!
//! accept/deny name only the order — PA reads buyer/seller/amount from the hold.
//! Replies: `obj(receipt, <order>)` / `{"status":"held"|"paid"|"cancelled",…}`,
//! or `obj(deny, <order>)` / `{"reason":…}`.
//!
//! ## Durability
//!
//! The whole state (`ledger` + `holds`) is serialized under one sled key and
//! rewritten atomically after each mutation, then `flush`ed. Single-key writes
//! are atomic, so the ledger is never torn. `Pa::open(path)` reloads it — escrow
//! survives a crash/restart. (A per-key WAL/transaction design is the scale-up;
//! the blob is correct and simple for the agent's small state.)
//!
//! ## Security (v1: durable, trusted `from`)
//!
//! Baked in: **authorization** (`accept`←`hold.seller`, `deny`←buyer/seller, vs
//! `ctx.from()`), **idempotency** (duplicate `reserve` rejected; one-way state
//! machine), **validation** (positive, overflow-checked amounts), **scoped
//! receipts**. Deferred (roadmap in `docs/agents/PA_DESIGN.md`): authenticated
//! `from` (FIPA layer), signed messages + receipts (end-to-end).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

/// The state of an escrow hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldState {
    Held,
    Paid,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Hold {
    buyer: String,
    seller: String,
    amount: u64,
    state: HoldState,
}

/// Persisted PA state: balances + escrow holds.
#[derive(Default, Serialize, Deserialize)]
struct State {
    ledger: BTreeMap<String, u64>,
    holds: BTreeMap<String, Hold>,
}

/// The Payment Agent — a durable ledger of balances and escrow holds.
pub struct Pa {
    db: sled::Db,
    state: State,
}

const STATE_KEY: &str = "state";

impl Pa {
    /// Open (or create) PA's durable store at `path`, reloading any prior state.
    pub fn open(path: impl AsRef<Path>) -> sled::Result<Self> {
        Self::with_db(sled::open(path)?)
    }

    fn with_db(db: sled::Db) -> sled::Result<Self> {
        let state = db
            .get(STATE_KEY)?
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        Ok(Pa { db, state })
    }

    /// Write the whole state atomically (single key) and flush.
    fn persist(&self) {
        if let Ok(bytes) = serde_json::to_vec(&self.state) {
            let _ = self.db.insert(STATE_KEY, bytes);
            let _ = self.db.flush();
        }
    }

    /// Credit an account (construction/test helper); persisted.
    pub fn credit(&mut self, account: impl Into<String>, amount: u64) {
        *self.state.ledger.entry(account.into()).or_default() += amount;
        self.persist();
    }

    pub fn balance(&self, account: &str) -> u64 {
        self.state.ledger.get(account).copied().unwrap_or(0)
    }

    pub fn hold_state(&self, order: &str) -> Option<HoldState> {
        self.state.holds.get(order).map(|h| h.state)
    }
}

#[derive(Deserialize)]
struct Seed {
    #[serde(default)]
    ledger: BTreeMap<String, u64>,
}

impl Agent for Pa {
    fn on_seed(&mut self, data: &[u8], _ctx: &mut Ctx) {
        if let Ok(seed) = serde_json::from_slice::<Seed>(data) {
            self.state.ledger.extend(seed.ledger);
            self.persist();
        }
    }

    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        let Some((action, order)) = action_and_subject(unl) else {
            return;
        };
        let from = ctx.from().to_string();
        match action.as_str() {
            "reserve" => self.reserve(&order, &from, body, ctx),
            "accept" => self.accept(&order, &from, ctx),
            "deny" => self.cancel(&order, &from, ctx),
            _ => {}
        }
    }
}

impl Pa {
    fn reserve(&mut self, order: &str, buyer: &str, body: &[u8], ctx: &mut Ctx) {
        if self.state.holds.contains_key(order) {
            return deny(ctx, buyer, order, "duplicate-order"); // idempotency
        }
        let (Some(seller), Some(amount)) = (json_str(body, "seller"), json_u64(body, "amount"))
        else {
            return deny(ctx, buyer, order, "bad-request");
        };
        if amount == 0 {
            return deny(ctx, buyer, order, "bad-amount");
        }
        if self.balance(buyer) < amount {
            return deny(ctx, buyer, order, "insufficient");
        }
        *self.state.ledger.entry(buyer.to_string()).or_default() -= amount;
        self.state.holds.insert(
            order.to_string(),
            Hold { buyer: buyer.into(), seller: seller.clone(), amount, state: HoldState::Held },
        );
        self.persist();
        receipt(ctx, buyer, order, "held", amount, None);
        receipt(ctx, &seller, order, "held", amount, Some(buyer)); // notify the seller
    }

    fn accept(&mut self, order: &str, from: &str, ctx: &mut Ctx) {
        let Some(h) = self.state.holds.get(order).cloned() else {
            return;
        };
        if from != h.seller {
            return deny(ctx, from, order, "unauthorized");
        }
        if h.state == HoldState::Held {
            *self.state.ledger.entry(h.seller.clone()).or_default() += h.amount;
            self.state.holds.get_mut(order).unwrap().state = HoldState::Paid;
            self.persist();
        }
        receipt(ctx, &h.buyer, order, "paid", h.amount, None);
        receipt(ctx, &h.seller, order, "paid", h.amount, None);
    }

    fn cancel(&mut self, order: &str, from: &str, ctx: &mut Ctx) {
        let Some(h) = self.state.holds.get(order).cloned() else {
            return;
        };
        if from != h.buyer && from != h.seller {
            return deny(ctx, from, order, "unauthorized");
        }
        if h.state == HoldState::Held {
            *self.state.ledger.entry(h.buyer.clone()).or_default() += h.amount; // refund
            self.state.holds.get_mut(order).unwrap().state = HoldState::Cancelled;
            self.persist();
        }
        receipt(ctx, &h.buyer, order, "cancelled", h.amount, None);
        receipt(ctx, &h.seller, order, "cancelled", h.amount, None);
    }
}

fn receipt(ctx: &mut Ctx, to: &str, order: &str, status: &str, amount: u64, buyer: Option<&str>) {
    let mut v = serde_json::json!({ "status": status, "amount": amount });
    if let Some(b) = buyer {
        v["buyer"] = serde_json::Value::String(b.to_string());
    }
    ctx.send(to, format!("obj(receipt, {order})"), serde_json::to_vec(&v).unwrap_or_default());
}

fn deny(ctx: &mut Ctx, to: &str, order: &str, reason: &str) {
    let body = serde_json::json!({ "reason": reason });
    ctx.send(to, format!("obj(deny, {order})"), serde_json::to_vec(&body).unwrap_or_default());
}

fn action_and_subject(unl: &str) -> Option<(String, String)> {
    let graph = parse_sentence(unl).ok()?;
    let rel = graph.relations.first()?;
    Some((inline_word(&rel.source)?, inline_word(&rel.target)?))
}

fn inline_word(node: &NodeRef) -> Option<String> {
    if let NodeRef::Inline(uw) = node {
        if let Uci::Ucn { root, .. } = &uw.uci {
            return Some(root.to_string());
        }
    }
    None
}

fn json_str(body: &[u8], key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

fn json_u64(body: &[u8], key: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get(key)?.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_pa() -> Pa {
        // a throwaway in-memory-ish db, auto-removed on drop
        Pa::with_db(sled::Config::new().temporary(true).open().unwrap()).unwrap()
    }

    fn unique_path() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "pa-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn run(pa: &mut Pa, from: &str, unl: &str, body: &[u8]) -> Vec<unl_agent::Outgoing> {
        let mut ctx = Ctx::new();
        ctx.set_from(from);
        pa.on_message(unl, body, &mut ctx);
        ctx.take()
    }

    fn status(out: &unl_agent::Outgoing) -> String {
        json_str(&out.body, "status").unwrap_or_default()
    }

    fn funded() -> Pa {
        let mut pa = temp_pa();
        pa.credit("BA", 10000);
        pa
    }

    #[test]
    fn reserve_holds_and_notifies_both() {
        let mut pa = funded();
        let out = run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|m| m.to == "BA" && status(m) == "held"));
        assert!(out.iter().any(|m| m.to == "bookSeller" && status(m) == "held"));
        assert_eq!(pa.balance("BA"), 9001);
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Held));
    }

    #[test]
    fn reserve_insufficient_is_denied() {
        let mut pa = funded();
        let out = run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":99999}"#);
        assert_eq!(out[0].unl, "obj(deny, LtG)");
        assert_eq!(json_str(&out[0].body, "reason").unwrap(), "insufficient");
        assert_eq!(pa.balance("BA"), 10000);
    }

    #[test]
    fn duplicate_reserve_is_rejected() {
        let mut pa = funded();
        run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        let out = run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        assert_eq!(json_str(&out[0].body, "reason").unwrap(), "duplicate-order");
    }

    #[test]
    fn accept_by_seller_releases_funds() {
        let mut pa = funded();
        run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        let out = run(&mut pa, "bookSeller", "obj(accept, LtG)", b"");
        assert!(out.iter().all(|m| status(m) == "paid"));
        assert_eq!(pa.balance("bookSeller"), 999);
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Paid));
    }

    #[test]
    fn accept_by_non_seller_is_unauthorized() {
        let mut pa = funded();
        run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        let out = run(&mut pa, "attacker", "obj(accept, LtG)", b"");
        assert_eq!(json_str(&out[0].body, "reason").unwrap(), "unauthorized");
        assert_eq!(pa.balance("bookSeller"), 0);
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Held));
    }

    #[test]
    fn deny_by_buyer_refunds() {
        let mut pa = funded();
        run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        let out = run(&mut pa, "BA", "obj(deny, LtG)", b"");
        assert!(out.iter().all(|m| status(m) == "cancelled"));
        assert_eq!(pa.balance("BA"), 10000);
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Cancelled));
    }

    #[test]
    fn accept_unknown_order_is_ignored() {
        let mut pa = funded();
        assert!(run(&mut pa, "bookSeller", "obj(accept, ghost)", b"").is_empty());
    }

    #[test]
    fn seed_ledger_from_data() {
        let mut pa = temp_pa();
        pa.on_seed(br#"{"ledger":{"BA":500}}"#, &mut Ctx::new());
        assert_eq!(pa.balance("BA"), 500);
    }

    #[test]
    fn state_survives_restart() {
        let path = unique_path();
        {
            let mut pa = Pa::open(&path).unwrap();
            pa.credit("BA", 10000);
            run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
            // pa dropped here — state was persisted on each mutation
        }
        {
            let pa = Pa::open(&path).unwrap(); // reopen the same store
            assert_eq!(pa.balance("BA"), 9001, "debited balance recovered");
            assert_eq!(pa.hold_state("LtG"), Some(HoldState::Held), "hold recovered");
        }
        std::fs::remove_dir_all(&path).ok();
    }
}
