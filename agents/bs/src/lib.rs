//! # Seller Agent (BS) — catalog + fulfilment around PA's escrow
//!
//! BS answers catalog queries, and around a purchase mirrors PA's escrow: on
//! PA's `held` notice it reserves the book and `accept`s payment; on `paid` it
//! ships to the buyer; on `cancelled` it releases the reservation.
//!
//! | in `unl` / `body` | BS does |
//! |---|---|
//! | `obj(catalog, <topic>)` | reply `obj(catalog, <topic>)` + `[{title,price}…]` to the asker |
//! | `obj(receipt, <order>)` `{status:"held", buyer}` | reserve; `obj(accept, <order>)` → pa |
//! | `obj(receipt, <order>)` `{status:"paid"}` | `obj(deliver, <order>)` → buyer (ship) |
//! | `obj(receipt, <order>)` `{status:"cancelled"}` | release the reservation |
//!
//! BS replies to `ctx.from()` for catalog and addresses `pa` (an alias the node
//! resolves) and the buyer (by id) for the rest — transport-agnostic.

use std::collections::BTreeMap;

use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

/// The seller. `has_ltg` controls whether "Limits to Growth" is in the catalog
/// (so a node can simulate a seller that lacks the book).
pub struct Seller {
    has_ltg: bool,
    buyers: BTreeMap<String, String>, // order -> buyer id
}

impl Seller {
    pub fn new(has_ltg: bool) -> Self {
        Seller { has_ltg, buyers: BTreeMap::new() }
    }
}

impl Default for Seller {
    fn default() -> Self {
        Self::new(true)
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
                        self.buyers.insert(subject.clone(), buyer); // reserve the book
                        ctx.send("pa", format!("obj(accept, {subject})"), Vec::new());
                    }
                }
                "paid" => {
                    if let Some(buyer) = self.buyers.get(&subject) {
                        ctx.send(buyer.clone(), format!("obj(deliver, {subject})"), Vec::new()); // ship
                    }
                }
                "cancelled" => {
                    self.buyers.remove(&subject); // release the reservation
                }
                _ => {}
            },
            _ => {}
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bs: &mut Seller, from: &str, unl: &str, body: &[u8]) -> Vec<unl_agent::Outgoing> {
        let mut ctx = Ctx::new();
        ctx.set_from(from);
        bs.on_message(unl, body, &mut ctx);
        ctx.take()
    }

    #[test]
    fn catalog_lists_ltg() {
        let mut bs = Seller::new(true);
        let out = run(&mut bs, "BA", "obj(catalog, systemdynamics)", b"");
        assert_eq!(out[0].to, "BA");
        assert!(String::from_utf8_lossy(&out[0].body).contains("LtG"));
    }

    #[test]
    fn held_reserves_and_accepts() {
        let mut bs = Seller::new(true);
        let out = run(&mut bs, "pa", "obj(receipt, LtG)", br#"{"status":"held","buyer":"BA"}"#);
        assert_eq!(out[0].to, "pa");
        assert_eq!(out[0].unl, "obj(accept, LtG)");
    }

    #[test]
    fn paid_ships_to_buyer() {
        let mut bs = Seller::new(true);
        run(&mut bs, "pa", "obj(receipt, LtG)", br#"{"status":"held","buyer":"BA"}"#);
        let out = run(&mut bs, "pa", "obj(receipt, LtG)", br#"{"status":"paid"}"#);
        assert_eq!(out[0].to, "BA");
        assert_eq!(out[0].unl, "obj(deliver, LtG)");
    }
}
