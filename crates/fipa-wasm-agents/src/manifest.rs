//! The agent **manifest** (the bundle `HEAD`) + load-time **gating**
//! (`AGENT_HOST_ABI.md` §3–§5, §10).
//!
//! The manifest is the agent's resource-management record, read first. A node
//! [`NodeProfile`] declares which [`Capability`]s it offers and the budget ceilings
//! it allows; [`NodeProfile::fit`] is the **load-time fit** (operator-facing): it
//! either returns the effective [`Grant`] (core caps + the requested caps the
//! profile offers, with budgets clamped) or a precise [`FitError`]. Once admitted,
//! ungranted *runtime* calls return a uniform `denied` (the agent cannot tell which
//! reason) — that part lands with the capabilities in M3+.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A gated agent capability — the family of host-calls an agent may use.
/// [`Capability::Messaging`] and [`Capability::Log`] are **core** (every admitted
/// agent gets them); the rest are opt-in and must be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Messaging,
    Log,
    Discovery,
    State,
    Time,
    Llm,
    Crypto,
    Spawn,
}

impl Capability {
    /// Core capabilities every admitted agent receives regardless of `grants`.
    pub fn is_core(self) -> bool {
        matches!(self, Capability::Messaging | Capability::Log)
    }
}

/// Which node shape an agent targets (advisory metadata; the real gate is the
/// capability + budget fit in [`NodeProfile::fit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Normal,
    Iot,
    Either,
}

/// The brain that runs the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brain {
    Wasm,
    Native,
    Llm,
}

/// Resource budgets the node enforces (the canonical source; per-engine caps like
/// `proto::AgentCapabilities` are derived from this).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub mem_kb: u64,
    pub fuel: u64,
    pub state_kb: u64,
    pub timers: u32,
    pub msg_per_s: u32,
    /// Network scope: `"none"` | `"platform"` | `"any"` | `"node:<id>,…"`.
    pub net: String,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            mem_kb: 4096,
            fuel: 100_000_000,
            state_kb: 256,
            timers: 4,
            msg_per_s: 50,
            net: "platform".into(),
        }
    }
}

/// The agent manifest — the bundle `HEAD`. Extends the identity header with the
/// profile, brain, requested grants, and budgets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "type")]
    pub type_id: Uuid,
    pub desc: String,
    #[serde(default)]
    pub name: Option<String>,
    pub profile: Profile,
    pub brain: Brain,
    #[serde(default)]
    pub grants: Vec<Capability>,
    #[serde(default)]
    pub budget: Budget,
}

impl Manifest {
    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// The effective, post-fit authority granted to an admitted agent.
#[derive(Clone, Debug)]
pub struct Grant {
    pub caps: HashSet<Capability>,
    pub budget: Budget,
}

impl Grant {
    /// Whether a capability is granted (the gate every host-call consults). A `false`
    /// here is the uniform, opaque `denied` at runtime (`AGENT_HOST_ABI.md` §10).
    pub fn granted(&self, cap: Capability) -> bool {
        self.caps.contains(&cap)
    }

    /// Full authority — used for native/infrastructure agents (DF/AMS/PA), which are
    /// trusted, host-instantiated templates rather than gated tenant code.
    pub fn full() -> Self {
        use Capability::*;
        Grant {
            caps: [Messaging, Log, Discovery, State, Time, Llm, Crypto, Spawn].into_iter().collect(),
            budget: Budget::default(),
        }
    }
}

/// Why a manifest does not fit a node profile (load-time, operator-facing — the
/// agent never sees these; it is simply not admitted).
#[derive(Clone, Debug, PartialEq)]
pub enum FitError {
    /// The agent requested a capability this node profile does not offer.
    Ungranted(Capability),
    /// A budget exceeds this profile's ceiling.
    OverBudget(&'static str),
}

/// What a node profile provides: the capability set + budget ceilings.
#[derive(Clone, Debug)]
pub struct NodeProfile {
    pub profile: Profile,
    pub caps: HashSet<Capability>,
    pub ceiling: Budget,
}

impl NodeProfile {
    /// A full-featured node: every capability, generous ceilings.
    pub fn normal() -> Self {
        use Capability::*;
        NodeProfile {
            profile: Profile::Normal,
            caps: [Messaging, Log, Discovery, State, Time, Llm, Crypto, Spawn].into_iter().collect(),
            ceiling: Budget {
                mem_kb: 1 << 20,        // 1 GiB
                fuel: u64::MAX,
                state_kb: 1 << 20,      // 1 GiB
                timers: 1024,
                msg_per_s: 100_000,
                net: "any".into(),
            },
        }
    }

    /// A constrained edge node: no `Llm`, no `Spawn`; tiny budgets.
    pub fn iot() -> Self {
        use Capability::*;
        NodeProfile {
            profile: Profile::Iot,
            caps: [Messaging, Log, Discovery, State, Time, Crypto].into_iter().collect(),
            ceiling: Budget {
                mem_kb: 512,
                fuel: 1_000_000,
                state_kb: 64,
                timers: 4,
                msg_per_s: 50,
                net: "platform".into(),
            },
        }
    }

    /// **Load-time fit.** Returns the effective [`Grant`] (core caps + the requested
    /// caps this profile offers, with the agent's budget), or a precise [`FitError`].
    /// The capability + budget check *is* the profile gate; `Manifest::profile` is
    /// advisory.
    pub fn fit(&self, m: &Manifest) -> Result<Grant, FitError> {
        for &c in &m.grants {
            if !self.caps.contains(&c) {
                return Err(FitError::Ungranted(c));
            }
        }
        let b = &m.budget;
        if b.mem_kb > self.ceiling.mem_kb {
            return Err(FitError::OverBudget("mem_kb"));
        }
        if b.fuel > self.ceiling.fuel {
            return Err(FitError::OverBudget("fuel"));
        }
        if b.state_kb > self.ceiling.state_kb {
            return Err(FitError::OverBudget("state_kb"));
        }
        if b.timers > self.ceiling.timers {
            return Err(FitError::OverBudget("timers"));
        }
        if b.msg_per_s > self.ceiling.msg_per_s {
            return Err(FitError::OverBudget("msg_per_s"));
        }
        let mut caps: HashSet<Capability> = m.grants.iter().copied().collect();
        caps.insert(Capability::Messaging); // core
        caps.insert(Capability::Log); // core
        Ok(Grant { caps, budget: m.budget.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(grants: &[Capability], budget: Budget) -> Manifest {
        Manifest {
            type_id: Uuid::nil(),
            desc: "test".into(),
            name: None,
            profile: Profile::Either,
            brain: Brain::Wasm,
            grants: grants.to_vec(),
            budget,
        }
    }

    #[test]
    fn normal_node_grants_requested_plus_core() {
        let g = NodeProfile::normal()
            .fit(&manifest(&[Capability::Discovery, Capability::State, Capability::Time], Budget::default()))
            .unwrap();
        assert!(g.granted(Capability::Discovery));
        assert!(g.granted(Capability::State));
        assert!(g.granted(Capability::Messaging)); // core, always granted
        assert!(g.granted(Capability::Log)); // core
        assert!(!g.granted(Capability::Llm)); // not requested → not granted
    }

    #[test]
    fn iot_node_refuses_an_unoffered_capability() {
        let err = NodeProfile::iot().fit(&manifest(&[Capability::Llm], Budget::default())).unwrap_err();
        assert_eq!(err, FitError::Ungranted(Capability::Llm));
    }

    #[test]
    fn over_budget_is_rejected_with_the_offending_field() {
        let big = Budget { mem_kb: 999_999, ..Budget::default() };
        let err = NodeProfile::iot().fit(&manifest(&[Capability::State], big)).unwrap_err();
        assert_eq!(err, FitError::OverBudget("mem_kb"));
    }

    #[test]
    fn manifest_json_roundtrips() {
        let m = manifest(&[Capability::Discovery, Capability::Llm], Budget::default());
        let back = Manifest::from_json(&m.to_json()).unwrap();
        assert_eq!(back.grants, m.grants);
        assert_eq!(back.brain, Brain::Wasm);
        assert_eq!(back.profile, Profile::Either);
    }
}
