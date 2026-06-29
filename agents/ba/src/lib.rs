//! # Buyer Agent (BA) — the book-buy conversation (mobile / wasm)
//!
//! BA drives the whole purchase as a small state machine: find a seller (DF),
//! resolve it (AMS), browse (BS), reserve payment (PA), and take delivery. It is
//! the **mobile** agent — written once here and compiled to **wasm32**
//! (sandboxed, migratable) via [`unl_agent::export_agent!`]; the same code also
//! builds native (rlib) so a node can run it in-process.
//!
//! Conversation (replies dispatched by the UNL verb + BA's state):
//! ```text
//! start    → seek bookselling → df
//! provide  → locate <seller>  → ams
//! at       → catalog          → <seller>
//! catalog  → reserve LtG       → pa
//! receipt(held)  → await delivery
//! deliver  → result: bought
//! deny / empty → result: failed (with the reason)
//! ```
//! BA emits its final verdict to `result` (a non-agent), where the node picks it
//! up. BA never needs `ctx.from()` — it addresses fixed roles and the seller it
//! learned from DF — so it works even before wasm sender-threading lands.

use serde::Deserialize;
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

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

/// The buyer agent.
pub struct Buyer {
    st: St,
    seller: String,
}

impl Default for Buyer {
    fn default() -> Self {
        Buyer { st: St::Init, seller: String::new() }
    }
}

impl Buyer {
    pub fn new() -> Self {
        Self::default()
    }

    fn fail(&mut self, ctx: &mut Ctx, why: &str) {
        ctx.send("result", "obj(failed, x)", why.as_bytes().to_vec());
        self.st = St::Failed;
    }
}

impl Agent for Buyer {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        let Some((verb, _subject)) = verb_subject(unl) else { return };

        // Any deny aborts the purchase, whatever state we're in.
        if verb == "deny" {
            let reason = jfield(body, "reason").unwrap_or_else(|| "denied".into());
            return self.fail(ctx, &reason);
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
                        // the seller is a UUID → ask AMS with it in the body.
                        let q = serde_json::json!({ "agent": p });
                        ctx.send("ams", "obj(locate, agent)", serde_json::to_vec(&q).unwrap());
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
                    self.st = St::Delivery; // payment secured; await the book
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

// Export the wasm ABI when built for wasm32 (no-op on a native build).
unl_agent::export_agent!(Buyer::new());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_seeks_a_bookseller() {
        let mut ba = Buyer::new();
        let mut ctx = Ctx::new();
        ba.on_message("obj(start, buy)", b"", &mut ctx);
        let out = ctx.take();
        assert_eq!(out[0].to, "df");
        assert_eq!(out[0].unl, "obj(seek, bookselling)");
    }

    #[test]
    fn deny_aborts_with_reason() {
        let mut ba = Buyer::new();
        let mut ctx = Ctx::new();
        ba.on_message("obj(deny, LtG)", br#"{"reason":"insufficient"}"#, &mut ctx);
        let out = ctx.take();
        assert_eq!(out[0].to, "result");
        assert_eq!(out[0].body, b"insufficient");
    }
}
