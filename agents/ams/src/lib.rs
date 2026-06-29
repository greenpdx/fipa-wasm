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
//! `(from, unl, body)` — UNL carries only the **action verb** (a placeholder
//! subject `agent`); the agent in question is a **UUID**, which is structured
//! machine data and so travels in the **JSON body**, never in the UNL graph (UNL
//! stays human/semantic). The body also carries the address / referral target.
//!
//! ## Actions
//!
//! | Action | in: `unl` / `body` | AMS does | reply → `from`: `unl` / `body` |
//! |---|---|---|---|
//! | bind   | `obj(bind, agent)` / `{"agent":"<uuid>","address":"<addr>"}` | store `<uuid> → addr` | `obj(bound, agent)` / — |
//! | locate | `obj(locate, agent)` / `{"agent":"<uuid>"}` | resolve | `obj(at, agent)` / `{"agent","address"}` **or** `obj(refer, agent)` / `{"agent","ams"}` |
//!
//! An `at` reply with no `address` means not-found (unknown agent, no upstream).
//! Each reply echoes `"agent"` so a resolver chasing referrals can correlate.
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
        let Some(action) = action_verb(unl) else {
            return;
        };
        let from = ctx.from().to_string();
        match action.as_str() {
            "bind" => {
                // R3: an agent binds only itself — the authenticated sender must be
                // the agent being bound, else the bind is refused (THREAT_MODEL C2).
                match (json_field(body, "agent"), json_field(body, "address")) {
                    (Some(agent), Some(addr)) if agent == from => {
                        self.records.insert(agent, addr);
                        ctx.send(from, "obj(bound, agent)", Vec::new());
                    }
                    _ => {
                        ctx.send(from, "obj(refuse, agent)", Vec::new());
                    }
                }
            }
            "locate" => {
                let Some(agent) = json_field(body, "agent") else {
                    return;
                };
                if let Some(addr) = self.records.get(&agent) {
                    // "I have it right here" (authoritative or cached).
                    let b = serde_json::json!({ "agent": agent, "address": addr });
                    ctx.send(from, "obj(at, agent)", serde_json::to_vec(&b).unwrap_or_default());
                } else if let Some(up) = &self.upstream {
                    // "check XYZ AMS" (referral; the node may chase it recursively).
                    let b = serde_json::json!({ "agent": agent, "ams": up });
                    ctx.send(from, "obj(refer, agent)", serde_json::to_vec(&b).unwrap_or_default());
                } else {
                    // not found: no address field
                    let b = serde_json::json!({ "agent": agent });
                    ctx.send(from, "obj(at, agent)", serde_json::to_vec(&b).unwrap_or_default());
                }
            }
            _ => {}
        }
    }
}

/// Parse `obj(<action>, agent)` → the action verb (the subject is a placeholder).
fn action_verb(unl: &str) -> Option<String> {
    let graph = parse_sentence(unl).ok()?;
    let rel = graph.relations.first()?;
    inline_word(&rel.source)
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

    // The agent is a UUID; here a short stand-in "S" carried in the body.
    #[test]
    fn bind_then_locate_is_direct() {
        let mut ams = Ams::new();
        let out = run(&mut ams, "S", "obj(bind, agent)", br#"{"agent":"S","address":"127.0.0.1:9001"}"#);
        assert_eq!(out[0].unl, "obj(bound, agent)");

        let out = run(&mut ams, "BA", "obj(locate, agent)", br#"{"agent":"S"}"#);
        assert_eq!(out[0].to, "BA");
        assert_eq!(out[0].unl, "obj(at, agent)");
        assert_eq!(json_field(&out[0].body, "agent").unwrap(), "S"); // echoed for correlation
        assert_eq!(json_field(&out[0].body, "address").unwrap(), "127.0.0.1:9001");
    }

    #[test]
    fn bind_of_another_agent_is_refused() {
        // R3: sender "attacker" tries to bind victim "V" → refused, no record.
        let mut ams = Ams::new();
        let out = run(&mut ams, "attacker", "obj(bind, agent)", br#"{"agent":"V","address":"6.6.6.6:9000"}"#);
        assert_eq!(out[0].unl, "obj(refuse, agent)");
        assert!(ams.address("V").is_none());
    }

    #[test]
    fn locate_unknown_with_upstream_refers() {
        let mut ams = Ams::new();
        ams.set_upstream("ams-root");
        let out = run(&mut ams, "BA", "obj(locate, agent)", br#"{"agent":"S"}"#);
        assert_eq!(out[0].unl, "obj(refer, agent)");
        assert_eq!(json_field(&out[0].body, "ams").unwrap(), "ams-root");
    }

    #[test]
    fn locate_unknown_no_upstream_is_not_found() {
        let mut ams = Ams::new();
        let out = run(&mut ams, "BA", "obj(locate, agent)", br#"{"agent":"ghost"}"#);
        assert_eq!(out[0].unl, "obj(at, agent)");
        assert!(json_field(&out[0].body, "address").is_none()); // no address ⇒ not found
    }

    #[test]
    fn seed_from_data_block() {
        let mut ams = Ams::new();
        ams.on_seed(br#"{"records":{"bookSeller":"10.0.0.1:9001"},"upstream":"ams-root"}"#, &mut Ctx::new());
        assert_eq!(ams.address("bookSeller"), Some("10.0.0.1:9001"));
    }
}
