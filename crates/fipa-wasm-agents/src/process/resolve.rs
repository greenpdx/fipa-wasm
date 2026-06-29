//! AMS name resolution — the **FIPA-layer recursion**.
//!
//! AMS *agents* only ever answer directly (`at`) or refer (`refer`) to an
//! upstream AMS. Turning a chain of referrals into a final address is done here,
//! in the node: [`resolve`] asks an AMS, and on a referral either returns it
//! (iterative) or follows it (recursive). The agents stay simple; the multi-hop
//! logic is one place.
//!
//! *Which mode to use — iterative or recursive — is a policy decision left for
//! later; for now the caller passes it explicitly.*

use std::collections::HashMap;

use unl_core::{NodeRef, Uci};
use unl_parser::parse_sentence;

use crate::wasm::AgentRuntime;

/// The outcome of a resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Resolved to an address.
    Found(String),
    /// A referral to another AMS (iterative mode only).
    Referral(String),
    /// Unknown agent / dead end.
    NotFound,
}

/// Resolve `agent` starting at AMS `start`, asking the AMS runtimes in `amses`.
/// `recursive` chases referrals to a final address; otherwise the first referral
/// is returned. Bounded by `max_hops` (referral-loop guard).
pub fn resolve(
    amses: &mut HashMap<String, Box<dyn AgentRuntime>>,
    start: &str,
    agent: &str,
    recursive: bool,
    max_hops: usize,
) -> Resolution {
    let mut current = start.to_string();
    for _ in 0..max_hops {
        let Some(ams) = amses.get_mut(&current) else {
            return Resolution::NotFound;
        };
        // The agent is a UUID → it travels in the body, not the UNL.
        let body = serde_json::json!({ "agent": agent }).to_string();
        if ams.config("resolver", b"obj(locate, agent)", body.as_bytes()).is_err() {
            return Resolution::NotFound;
        }
        let Some(reply) = ams.take_sends().into_iter().next() else {
            return Resolution::NotFound;
        };
        let text = String::from_utf8_lossy(&reply.unl);
        match reply_verb(&text).as_deref() {
            Some("at") => {
                return match json_field(&reply.body, "address") {
                    Some(addr) if !addr.is_empty() => Resolution::Found(addr),
                    _ => Resolution::NotFound, // empty {} ⇒ not found
                };
            }
            Some("refer") => {
                let Some(next) = json_field(&reply.body, "ams") else {
                    return Resolution::NotFound;
                };
                if !recursive {
                    return Resolution::Referral(next);
                }
                current = next; // chase
            }
            _ => return Resolution::NotFound,
        }
    }
    Resolution::NotFound // hop limit hit
}

fn reply_verb(unl: &str) -> Option<String> {
    let graph = parse_sentence(unl).ok()?;
    let rel = graph.relations.first()?;
    if let NodeRef::Inline(uw) = &rel.source {
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
    use crate::wasm::NativeRuntime;
    use ams_agent::Ams;

    // A two-level AMS: `leaf` refers up to `root`, which is authoritative.
    fn amses() -> HashMap<String, Box<dyn AgentRuntime>> {
        let mut root = Ams::new();
        root.bind("bookSeller", "127.0.0.1:9001");
        let mut leaf = Ams::new();
        leaf.set_upstream("root");

        let mut m: HashMap<String, Box<dyn AgentRuntime>> = HashMap::new();
        m.insert("root".into(), Box::new(NativeRuntime::new(root)));
        m.insert("leaf".into(), Box::new(NativeRuntime::new(leaf)));
        m
    }

    #[test]
    fn direct() {
        let mut m = amses();
        assert_eq!(
            resolve(&mut m, "root", "bookSeller", false, 8),
            Resolution::Found("127.0.0.1:9001".into())
        );
    }

    #[test]
    fn iterative_returns_the_referral() {
        let mut m = amses();
        assert_eq!(
            resolve(&mut m, "leaf", "bookSeller", false, 8),
            Resolution::Referral("root".into())
        );
    }

    #[test]
    fn recursive_chases_the_referral() {
        let mut m = amses();
        assert_eq!(
            resolve(&mut m, "leaf", "bookSeller", true, 8),
            Resolution::Found("127.0.0.1:9001".into())
        );
    }

    #[test]
    fn unknown_is_not_found() {
        let mut m = amses();
        assert_eq!(resolve(&mut m, "leaf", "ghost", true, 8), Resolution::NotFound);
    }
}
