//! # Agent Management System (AMS) — the white pages
//!
//! AMS resolves an **agent id → its address** (physical location) — the system's
//! DNS. Where DF answers *"who sells books?"* with an id (`bookSeller`), AMS
//! answers *"where is `bookSeller`?"* with an address.
//!
//! ```text
//! BA → AMS : locate bookSeller     AMS → BA : at bookSeller {address}
//! ```
//!
//! ## The agent does two of the three resolution modes
//!
//! When asked **"where is ABC?"**:
//! - **"I have it right here"** (authoritative or cached) → `at` + the address.
//! - **"check XYZ AMS"** (referral) → `refer` + the upstream AMS id.
//!
//! The **third mode — recursion** ("I'll check XYZ AMS and tell you") is a
//! **FIPA-layer** concern, *not* the agent's: a node-side resolver follows a
//! chain of referrals to a final address. The AMS agent therefore stays simple
//! and stateless about other AMSes — it only ever answers directly or refers.
//! *(Which mode a resolver uses — iterative vs recursive — is a policy decision
//! deferred for later.)*
//!
//! ## Message model
//!
//! `(from, unl, body)` — UNL carries the action + subject agent; JSON body
//! carries structured data (the address, the referral target).
//!
//! ## Actions
//!
//! | Action | in: `unl` / `body` | AMS does | reply → `from`: `unl` / `body` |
//! |---|---|---|---|
//! | bind   | `obj(bind, <agent>)` / `{"address":"<addr>"}` | store `<agent> → addr` | `obj(bound, <agent>)` / — |
//! | locate | `obj(locate, <agent>)` / — | resolve | `obj(at, <agent>)` / `{"address":…}` **or** `obj(refer, <agent>)` / `{"ams":…}` |
//!
//! An `at` with empty `{}` means not-found (unknown agent, no upstream).
//!
//! ## Data model
//!
//! `records: agent -> address` (authoritative bindings + answers cached by the
//! resolver via `bind`), `upstream: Option<ams-id>`. Seeded from the `DATA`
//! block: `{ "records": {…}, "upstream": "ams-root" }`. TTL on cache is a v2 hook.

use std::collections::BTreeMap;

use serde::Deserialize;
use unl_agent::{Agent, Ctx};
use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

/// The Agent Management System agent: an agent → address registry.
#[derive(Default)]
pub struct Ams {
    records: BTreeMap<String, String>,
    upstream: Option<String>,
}

impl Ams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind an address (authoritative). Construction/test helper.
    pub fn bind(&mut self, agent: impl Into<String>, address: impl Into<String>) {
        self.records.insert(agent.into(), address.into());
    }

    /// Set the upstream AMS used for referrals. Construction/test helper.
    pub fn set_upstream(&mut self, ams: impl Into<String>) {
        self.upstream = Some(ams.into());
    }

    pub fn address(&self, agent: &str) -> Option<&str> {
        self.records.get(agent).map(String::as_str)
    }
}

#[derive(Deserialize)]
struct Seed {
    #[serde(default)]
    records: BTreeMap<String, String>,
    #[serde(default)]
    upstream: Option<String>,
}

impl Agent for Ams {
    fn on_seed(&mut self, data: &[u8], _ctx: &mut Ctx) {
        if let Ok(seed) = serde_json::from_slice::<Seed>(data) {
            self.records.extend(seed.records);
            if seed.upstream.is_some() {
                self.upstream = seed.upstream;
            }
        }
    }

    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        let Some((action, agent)) = action_and_subject(unl) else {
            return;
        };
        let from = ctx.from().to_string();
        match action.as_str() {
            "bind" => {
                if let Some(addr) = json_field(body, "address") {
                    self.records.insert(agent.clone(), addr);
                }
                ctx.send(from, format!("obj(bound, {agent})"), Vec::new());
            }
            "locate" => {
                if let Some(addr) = self.records.get(&agent) {
                    // "I have it right here" (authoritative or cached).
                    let b = format!("{{\"address\":\"{addr}\"}}").into_bytes();
                    ctx.send(from, format!("obj(at, {agent})"), b);
                } else if let Some(up) = &self.upstream {
                    // "check XYZ AMS" (referral; the node may chase it recursively).
                    let b = format!("{{\"ams\":\"{up}\"}}").into_bytes();
                    ctx.send(from, format!("obj(refer, {agent})"), b);
                } else {
                    // not found
                    ctx.send(from, format!("obj(at, {agent})"), b"{}".to_vec());
                }
            }
            _ => {}
        }
    }
}

/// Parse `obj(<action>, <agent>)` → (action word, agent word).
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

fn json_field(body: &[u8], key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ams: &mut Ams, from: &str, unl: &str, body: &[u8]) -> Vec<unl_agent::Outgoing> {
        let mut ctx = Ctx::new();
        ctx.set_from(from);
        ams.on_message(unl, body, &mut ctx);
        ctx.take()
    }

    #[test]
    fn bind_then_locate_is_direct() {
        let mut ams = Ams::new();
        let out = run(&mut ams, "bookSeller", "obj(bind, bookSeller)", br#"{"address":"127.0.0.1:9001"}"#);
        assert_eq!(out[0].unl, "obj(bound, bookSeller)");

        let out = run(&mut ams, "BA", "obj(locate, bookSeller)", b"");
        assert_eq!(out[0].to, "BA");
        assert_eq!(out[0].unl, "obj(at, bookSeller)");
        assert_eq!(json_field(&out[0].body, "address").unwrap(), "127.0.0.1:9001");
    }

    #[test]
    fn locate_unknown_with_upstream_refers() {
        let mut ams = Ams::new();
        ams.set_upstream("ams-root");
        let out = run(&mut ams, "BA", "obj(locate, bookSeller)", b"");
        assert_eq!(out[0].unl, "obj(refer, bookSeller)");
        assert_eq!(json_field(&out[0].body, "ams").unwrap(), "ams-root");
    }

    #[test]
    fn locate_unknown_no_upstream_is_not_found() {
        let mut ams = Ams::new();
        let out = run(&mut ams, "BA", "obj(locate, ghost)", b"");
        assert_eq!(out[0].unl, "obj(at, ghost)");
        assert!(json_field(&out[0].body, "address").is_none()); // empty {}
    }

    #[test]
    fn seed_from_data_block() {
        let mut ams = Ams::new();
        ams.on_seed(br#"{"records":{"bookSeller":"10.0.0.1:9001"},"upstream":"ams-root"}"#, &mut Ctx::new());
        assert_eq!(ams.address("bookSeller"), Some("10.0.0.1:9001"));
    }
}
