//! In-node message router with **authenticated `from`**.
//!
//! An agent emits replies via `ctx.send(to, …)` → an [`OutboundIntent`] that
//! carries *no sender* — the sender is implicit (the agent that emitted it). The
//! router is what turns that into a delivered message, and it stamps `from` =
//! **the agent it just drained**, not anything the agent could claim. So an
//! agent **cannot forge its identity**: a malicious agent that "wants" to be
//! `bookSeller` still has its own id stamped on everything it sends, and PA's
//! authorization (`accept` only from the hold's seller) rejects it.
//!
//! Trust boundaries:
//! - **intra-node** (agent → agent): authentic by construction (the router knows
//!   which local agent emitted each message);
//! - **external injection** ([`Router::send`]): the trust boundary — a real node
//!   authenticates the external client (signed request / authenticated channel)
//!   *before* injecting with a claimed `from`;
//! - **cross-node** (remote agents): needs signed messages / authenticated
//!   channels between nodes — the next layer, not built here.

use std::collections::{HashMap, VecDeque};

use crate::identity::Aliases;
use crate::wasm::AgentRuntime;

/// A message in flight: `from` is the authenticated sender, stamped by the router.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub from: String,
    pub to: String,
    pub unl: Vec<u8>,
    pub body: Vec<u8>,
}

/// Routes messages between local agents, stamping the authenticated sender.
///
/// Agents are keyed by their canonical id (a UUID). A `to` may also be a
/// well-known **alias** (e.g. `"df"`) which is resolved to its UUID before
/// lookup, so bootstrap handles stay legible while identity stays a UUID.
#[derive(Default)]
pub struct Router {
    agents: HashMap<String, Box<dyn AgentRuntime>>,
    queue: VecDeque<Envelope>,
    aliases: Aliases,
    /// Messages addressed to non-local recipients (would go cross-node / to an
    /// external gateway). Useful for inspection in tests.
    pub outbox: Vec<Envelope>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a well-known `name` to an agent `id` (UUID): callers may address the
    /// `name`, and traces render the `id` as the `name`. Display + bootstrap only.
    pub fn bind_alias(&mut self, name: impl Into<String>, id: impl Into<String>) {
        self.aliases.bind(name, id);
    }

    /// Add a local agent under `id`, run its `init`, and queue any init output
    /// (stamped from `id`).
    pub fn add(&mut self, id: impl Into<String>, mut agent: Box<dyn AgentRuntime>) {
        let id = id.into();
        let _ = agent.init();
        for s in agent.take_sends() {
            self.queue.push_back(Envelope { from: id.clone(), to: s.receiver, unl: s.unl, body: s.body });
        }
        self.agents.insert(id, agent);
    }

    /// Inject an external message with a claimed `from`. This is the trust
    /// boundary — a real node authenticates the external client before this.
    pub fn send(&mut self, from: &str, to: &str, unl: &[u8], body: &[u8]) {
        self.queue.push_back(Envelope {
            from: from.into(),
            to: to.into(),
            unl: unl.to_vec(),
            body: body.to_vec(),
        });
    }

    /// Pump the queue until empty or `max_steps`. Each agent's emissions are
    /// stamped with **its own id** — the authenticated sender.
    pub fn run(&mut self, max_steps: usize) {
        let mut steps = 0;
        while let Some(env) = self.queue.pop_front() {
            if steps >= max_steps {
                break;
            }
            steps += 1;
            // Resolve a well-known alias `to` to its UUID; labels for the trace.
            let to = self.aliases.id_of(&env.to).map(str::to_string).unwrap_or_else(|| env.to.clone());
            let from_label = self.aliases.label(&env.from).to_string();
            let to_label = self.aliases.label(&to).to_string();
            let unl_text = String::from_utf8_lossy(&env.unl);
            match self.agents.get_mut(&to) {
                Some(agent) => {
                    crate::flow!("  {from_label} → {to_label} : {unl_text}");
                    if agent.config(&env.from, &env.unl, &env.body).is_err() {
                        continue;
                    }
                    // Authenticated: the agent that just processed is `to`;
                    // everything it sends is therefore *from* `to` (its UUID).
                    let sends = agent.take_sends();
                    for s in sends {
                        self.queue.push_back(Envelope {
                            from: to.clone(),
                            to: s.receiver,
                            unl: s.unl,
                            body: s.body,
                        });
                    }
                }
                None => {
                    crate::flow!("  {from_label} → {to_label} : {unl_text} (external/undelivered)");
                    self.outbox.push(env); // non-local recipient
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::NativeRuntime;
    use std::sync::atomic::{AtomicU64, Ordering};
    use unl_agent::{Agent, Ctx};

    /// On a message whose UNL contains `on`, emit a fixed `(to, unl, body)`.
    struct Trigger {
        on: String,
        to: String,
        unl: String,
        body: Vec<u8>,
    }
    impl Agent for Trigger {
        fn on_message(&mut self, unl: &str, _body: &[u8], ctx: &mut Ctx) {
            if unl.contains(&self.on) {
                ctx.send(&self.to, &self.unl, self.body.clone());
            }
        }
    }
    fn trigger(on: &str, to: &str, unl: &str) -> Box<dyn AgentRuntime> {
        Box::new(NativeRuntime::new(Trigger {
            on: on.into(),
            to: to.into(),
            unl: unl.into(),
            body: Vec::new(),
        }))
    }

    fn unique_path() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!("router-pa-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
    }

    fn funded_pa() -> Box<dyn AgentRuntime> {
        let mut pa = pa_agent::Pa::open(unique_path()).unwrap();
        pa.credit("BA", 10000);
        Box::new(NativeRuntime::new(pa))
    }

    fn json_status(e: &Envelope) -> String {
        serde_json::from_slice::<serde_json::Value>(&e.body)
            .ok()
            .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(str::to_string))
            .unwrap_or_default()
    }

    #[test]
    fn router_stamps_the_true_sender() {
        let mut r = Router::new();
        r.add("X", trigger("go", "out", "obj(ping, z)"));
        r.send("boot", "X", b"obj(go, x)", b""); // external trigger
        r.run(16);
        // X's emission to the non-local "out" carries from = "X" — not forgeable.
        let m = r.outbox.iter().find(|e| e.to == "out").expect("X emitted to out");
        assert_eq!(m.from, "X");
    }

    #[test]
    fn spoofed_accept_rejected_genuine_accepted() {
        let mut r = Router::new();
        r.add("pa", funded_pa());
        // Both try to `accept LtG` when poked — but the router stamps their real id.
        r.add("attacker", trigger("go", "pa", "obj(accept, LtG)"));
        r.add("seller", trigger("go", "pa", "obj(accept, LtG)"));

        // BA (external) reserves; seller = "seller", amount 999.
        r.send("BA", "pa", b"obj(reserve, LtG)", br#"{"seller":"seller","amount":999}"#);
        r.run(16);

        // Attacker pokes → emits accept, stamped from "attacker" ≠ seller → denied.
        r.send("boot", "attacker", b"obj(go, x)", b"");
        r.run(16);
        assert!(
            !r.outbox.iter().any(|e| e.to == "BA" && json_status(e) == "paid"),
            "a spoofed accept must not release funds"
        );

        // The genuine seller pokes → from "seller" → released, paid receipt to BA.
        r.send("boot", "seller", b"obj(go, x)", b"");
        r.run(16);
        assert!(
            r.outbox.iter().any(|e| e.to == "BA" && json_status(e) == "paid"),
            "the real seller's accept releases funds"
        );
    }
}
