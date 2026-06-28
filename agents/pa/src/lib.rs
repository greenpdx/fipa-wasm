//! # Payment Agent (PA) — escrow / hold settlement
//!
//! PA settles payment between a buyer and a seller with an **escrow hold**: it
//! **reserves** (holds) the buyer's funds, then **releases** them to the seller
//! on **accept** or back to the buyer on **deny**, issuing **receipts**
//! throughout. It is the money side of the book purchase.
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
//! - **reserve** (in): escrow funds for an order. **hold** is the resulting
//!   state. **accept** (in, seller): **release** to the seller. **deny** (in,
//!   buyer/seller): **release** back to the buyer — and PA's *out* `deny` is its
//!   rejection of a reserve. **receipt** (out): the proof, to both parties.
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
//! ## Security (v1: in-memory, trusted `from`)
//!
//! Baked in (correct now; enforceable once the node authenticates `from`):
//! - **authorization** — `accept` only from `hold.seller`; `deny` only from the
//!   hold's buyer or seller (checked against `ctx.from()`);
//! - **idempotency** — a duplicate `reserve` for an order is rejected; the
//!   one-way state machine blunts `accept`/`deny` replay;
//! - **validation** — amounts are non-negative integers, checked for overflow;
//! - **scoped receipts** — only to the hold's buyer/seller.
//!
//! Deferred (the security roadmap, in `docs/agents/PA_DESIGN.md`): authenticated
//! `from` (FIPA layer), signed messages + receipts (end-to-end), and **durable
//! ledger/holds** — without which a restart loses escrow. **Next step.**

use std::collections::BTreeMap;

use serde::Deserialize;
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

/// The state of an escrow hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldState {
    Held,
    Paid,
    Cancelled,
}

#[derive(Clone, Debug)]
struct Hold {
    buyer: String,
    seller: String,
    amount: u64,
    state: HoldState,
}

/// The Payment Agent: a ledger of balances and a set of escrow holds.
#[derive(Default)]
pub struct Pa {
    ledger: BTreeMap<String, u64>,
    holds: BTreeMap<String, Hold>,
}

impl Pa {
    pub fn new() -> Self {
        Self::default()
    }

    /// Credit an account (construction/test helper).
    pub fn credit(&mut self, account: impl Into<String>, amount: u64) {
        *self.ledger.entry(account.into()).or_default() += amount;
    }

    pub fn balance(&self, account: &str) -> u64 {
        self.ledger.get(account).copied().unwrap_or(0)
    }

    pub fn hold_state(&self, order: &str) -> Option<HoldState> {
        self.holds.get(order).map(|h| h.state)
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
            self.ledger.extend(seed.ledger);
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
        // idempotency: an order id is used once.
        if self.holds.contains_key(order) {
            return deny(ctx, buyer, order, "duplicate-order");
        }
        let (Some(seller), Some(amount)) = (json_str(body, "seller"), json_u64(body, "amount"))
        else {
            return deny(ctx, buyer, order, "bad-request");
        };
        // validation: a positive amount the buyer can cover.
        if amount == 0 {
            return deny(ctx, buyer, order, "bad-amount");
        }
        if self.balance(buyer) < amount {
            return deny(ctx, buyer, order, "insufficient");
        }
        *self.ledger.entry(buyer.to_string()).or_default() -= amount;
        self.holds.insert(
            order.to_string(),
            Hold { buyer: buyer.into(), seller: seller.clone(), amount, state: HoldState::Held },
        );
        // "held" receipt to the buyer, and notify the seller (PA confirms to BS).
        receipt(ctx, buyer, order, "held", amount, None);
        receipt(ctx, &seller, order, "held", amount, Some(buyer));
    }

    fn accept(&mut self, order: &str, from: &str, ctx: &mut Ctx) {
        let Some(h) = self.holds.get(order).cloned() else {
            return; // unknown order — ignore
        };
        // authorization: only the seller may accept (release funds to itself).
        if from != h.seller {
            return deny(ctx, from, order, "unauthorized");
        }
        if h.state == HoldState::Held {
            if let Some(bal) = self.ledger.entry(h.seller.clone()).or_default().checked_add(h.amount) {
                *self.ledger.get_mut(&h.seller).unwrap() = bal;
            }
            self.holds.get_mut(order).unwrap().state = HoldState::Paid;
        }
        receipt(ctx, &h.buyer, order, "paid", h.amount, None);
        receipt(ctx, &h.seller, order, "paid", h.amount, None);
    }

    fn cancel(&mut self, order: &str, from: &str, ctx: &mut Ctx) {
        let Some(h) = self.holds.get(order).cloned() else {
            return; // unknown order — ignore
        };
        // authorization: only a party to the hold may cancel.
        if from != h.buyer && from != h.seller {
            return deny(ctx, from, order, "unauthorized");
        }
        if h.state == HoldState::Held {
            *self.ledger.entry(h.buyer.clone()).or_default() += h.amount; // refund
            self.holds.get_mut(order).unwrap().state = HoldState::Cancelled;
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
        let mut pa = Pa::new();
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
        assert_eq!(pa.balance("BA"), 9001); // debited
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Held));
    }

    #[test]
    fn reserve_insufficient_is_denied() {
        let mut pa = funded();
        let out = run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":99999}"#);
        assert_eq!(out[0].unl, "obj(deny, LtG)");
        assert_eq!(json_str(&out[0].body, "reason").unwrap(), "insufficient");
        assert_eq!(pa.balance("BA"), 10000); // unchanged
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
        assert_eq!(out.len(), 2);
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
        assert_eq!(pa.balance("bookSeller"), 0); // no release
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Held));
    }

    #[test]
    fn deny_by_buyer_refunds() {
        let mut pa = funded();
        run(&mut pa, "BA", "obj(reserve, LtG)", br#"{"seller":"bookSeller","amount":999}"#);
        let out = run(&mut pa, "BA", "obj(deny, LtG)", b"");
        assert!(out.iter().all(|m| status(m) == "cancelled"));
        assert_eq!(pa.balance("BA"), 10000); // refunded
        assert_eq!(pa.hold_state("LtG"), Some(HoldState::Cancelled));
    }

    #[test]
    fn accept_unknown_order_is_ignored() {
        let mut pa = funded();
        let out = run(&mut pa, "bookSeller", "obj(accept, ghost)", b"");
        assert!(out.is_empty());
    }

    #[test]
    fn seed_ledger_from_data() {
        let mut pa = Pa::new();
        pa.on_seed(br#"{"ledger":{"BA":500}}"#, &mut Ctx::new());
        assert_eq!(pa.balance("BA"), 500);
    }
}
