//! # Directory Facilitator (DF) — the yellow pages
//!
//! Agents find each other by **what they do** (a *service*), not by name. A
//! provider **registers** the services it offers; a requester **searches** for
//! providers of a service. DF answers with matching provider id(s); the
//! requester then resolves a provider's address via **AMS**.
//!
//! In the book-buying flow:
//!
//! ```text
//! BA → DF : search "bookselling"     DF → BA : provider "bookSeller"
//! (then BA → AMS to resolve bookSeller's address)
//! ```
//!
//! ## Message model
//!
//! Every message is `(from, unl, body)`:
//! - `from` — the sender's id (`ctx.from()`); on register the provider is the
//!   sender (agents register *themselves*).
//! - `unl` — a **UNL graph** carrying the action verb + the service. UNL is the
//!   semantic part, so services can be matched by *meaning* (embedding) later,
//!   not just by exact name.
//! - `body` — optional **JSON** for the structured part (the provider-list
//!   result). UNL for *what it means*, JSON for *structured data*.
//!
//! ## Actions (v1: register + search)
//!
//! `<service>` is a UNL concept word (a single UW in v1, exact-matched; a richer
//! UNL graph matched by embedding similarity later).
//!
//! | Action | in: `unl` / `body` | DF does | reply → `from`: `unl` / `body` |
//! |---|---|---|---|
//! | register | `obj(offer, <service>)` / — | add `<service> → from` (idempotent) | `obj(registered, <service>)` / — |
//! | search   | `obj(seek, <service>)`  / — | look up providers of `<service>`   | `obj(provide, <service>)` / `["<id>", …]` |
//!
//! The action is the relation's **source** word (`offer`/`seek`); the service is
//! its **target** word. An empty search result is `[]`, not an error — the
//! requester decides whether to retry, ask a parent DF, or give up.
//!
//! ## Data model
//!
//! `registry: service -> {providers}`. Seeded once from the `DATA` block
//! (`on_seed`, JSON `{ "<service>": ["<provider>", …] }`) and grown by runtime
//! `register` messages.
//!
//! ## Expansion hooks (documented, not built in v1)
//!
//! - **Semantic match** — store a UNL *description* per entry and a vector index;
//!   `search` embeds the query and returns providers ranked by similarity. This
//!   is why DF is its own crate: it will pull an embedding/vector dependency that
//!   AMS/PA never see.
//! - **Federation** — a `parent: Option<AgentId>`; on a miss, forward the query
//!   to the parent DF and relay the answer, or reply with a redirect.
//! - **deregister / modify** — withdraw or update a registration.

use std::collections::{BTreeMap, BTreeSet};

use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

/// The Directory Facilitator agent: a service → providers registry.
#[derive(Default)]
pub struct Df {
    registry: BTreeMap<String, BTreeSet<String>>,
    // Federation hook (v2): on a search miss, forward to / redirect to a parent.
    // parent: Option<String>,
}

impl Df {
    pub fn new() -> Self {
        Df::default()
    }

    /// Providers of a service (sorted, possibly empty). Test/inspection helper.
    pub fn providers(&self, service: &str) -> Vec<String> {
        self.registry.get(service).into_iter().flatten().cloned().collect()
    }
}

impl Agent for Df {
    /// Seed the registry from the `DATA` block: JSON `{ "<service>": ["<id>"…] }`.
    fn on_seed(&mut self, data: &[u8], _ctx: &mut Ctx) {
        if let Ok(seed) = serde_json::from_slice::<BTreeMap<String, Vec<String>>>(data) {
            for (service, providers) in seed {
                self.registry.entry(service).or_default().extend(providers);
            }
        }
    }

    fn on_message(&mut self, unl: &str, _body: &[u8], ctx: &mut Ctx) {
        let Some((action, service)) = action_and_service(unl) else {
            return; // unparseable / not an obj(action, service) form — ignore
        };
        let from = ctx.from().to_string();
        match action.as_str() {
            // register: the sender offers a service.
            "offer" => {
                self.registry.entry(service.clone()).or_default().insert(from.clone());
                ctx.send(from, format!("obj(registered, {service})"), Vec::new());
            }
            // search: reply with the providers as a JSON array.
            "seek" => {
                let providers = self.providers(&service);
                let body = serde_json::to_vec(&providers).unwrap_or_default();
                ctx.send(from, format!("obj(provide, {service})"), body);
            }
            _ => {} // unknown verb — v1 ignores (not-understood is a v2 concern)
        }
    }
}

/// Parse `obj(<action>, <service>)` → (action word, service word).
fn action_and_service(unl: &str) -> Option<(String, String)> {
    let graph = parse_sentence(unl).ok()?;
    let rel = graph.relations.first()?;
    Some((inline_word(&rel.source)?, inline_word(&rel.target)?))
}

/// The lemma of an inline universal-word reference.
fn inline_word(node: &NodeRef) -> Option<String> {
    if let NodeRef::Inline(uw) = node {
        if let Uci::Ucn { root, .. } = &uw.uci {
            return Some(root.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(df: &mut Df, from: &str, unl: &str) -> Vec<unl_agent::Outgoing> {
        let mut ctx = Ctx::new();
        ctx.set_from(from);
        df.on_message(unl, b"", &mut ctx);
        ctx.take()
    }

    fn providers_in(out: &unl_agent::Outgoing) -> Vec<String> {
        serde_json::from_slice(&out.body).unwrap()
    }

    #[test]
    fn register_then_search() {
        let mut df = Df::new();

        let out = run(&mut df, "bookSeller", "obj(offer, bookselling)");
        assert_eq!(out[0].to, "bookSeller");
        assert_eq!(out[0].unl, "obj(registered, bookselling)");

        let out = run(&mut df, "BA", "obj(seek, bookselling)");
        assert_eq!(out[0].to, "BA");
        assert_eq!(out[0].unl, "obj(provide, bookselling)");
        assert_eq!(providers_in(&out[0]), vec!["bookSeller"]);
    }

    #[test]
    fn search_unknown_is_empty_not_error() {
        let mut df = Df::new();
        let out = run(&mut df, "BA", "obj(seek, nothing)");
        assert!(providers_in(&out[0]).is_empty());
    }

    #[test]
    fn register_is_idempotent() {
        let mut df = Df::new();
        run(&mut df, "bookSeller", "obj(offer, bookselling)");
        run(&mut df, "bookSeller", "obj(offer, bookselling)");
        let out = run(&mut df, "BA", "obj(seek, bookselling)");
        assert_eq!(providers_in(&out[0]).len(), 1);
    }

    #[test]
    fn seed_from_data_block() {
        let mut df = Df::new();
        df.on_seed(br#"{"bookselling":["bookSeller"]}"#, &mut Ctx::new());
        let out = run(&mut df, "BA", "obj(seek, bookselling)");
        assert_eq!(providers_in(&out[0]), vec!["bookSeller"]);
    }
}
