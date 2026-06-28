//! Profile-based runtime selection + a managed agent with restart (Phase 2).
//!
//! The node picks how an agent runs — [`Profile::InProcess`] (micro/IoT, or
//! trusted) or [`Profile::Hosted`] (its own resource-capped child) — and
//! [`build_runtime`] returns the matching [`AgentRuntime`]. [`ManagedAgent`]
//! wraps it with the recipe to rebuild and restarts on failure.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};

use super::{native_agent, AgentSpec, Limits, ProcessRuntime};
use crate::content::block::{BlockFile, TAG_WASM};
use crate::proto;
use crate::wasm::{AgentRuntime, OutboundIntent, WasmRuntime};

/// Where an agent runs.
#[derive(Clone, Debug)]
pub enum Profile {
    /// In the node's own process — micro/IoT, or trusted agents. No isolation
    /// beyond the in-process baseline (forbid-unsafe + catch_unwind).
    InProcess,
    /// In its own child process with OS resource caps — the isolated host
    /// profile (SIGKILLable, hard limits).
    Hosted(Limits),
}

/// Load a wasm bundle into an in-process runtime.
pub fn build_wasm(path: &Path) -> Result<Box<dyn AgentRuntime>> {
    let bytes = std::fs::read(path)?;
    let wasm = if BlockFile::is_block_container(&bytes) {
        BlockFile::decode(&bytes)?
            .get(TAG_WASM)
            .ok_or_else(|| anyhow!("bundle has no WASM block"))?
            .to_vec()
    } else {
        bytes
    };
    let caps = proto::AgentCapabilities {
        max_execution_time_ms: 1000,
        max_memory_bytes: 64 * 1024 * 1024,
        storage_quota_bytes: 1024 * 1024,
        ..Default::default()
    };
    Ok(Box::new(WasmRuntime::new(&wasm, &caps)?))
}

/// Build a runtime for `spec` per `profile`. In-process (Wasm/Native) or a child
/// process (Hosted). The node and the agent-host share this — the agent-host
/// always uses `InProcess`, because it *is* the isolated process.
pub fn build_runtime(
    spec: &AgentSpec,
    profile: &Profile,
    host_bin: &Path,
    timeout: Duration,
) -> Result<Box<dyn AgentRuntime>> {
    match profile {
        Profile::InProcess => match spec {
            AgentSpec::Native(name) => {
                native_agent(name).ok_or_else(|| anyhow!("unknown native agent: {name}"))
            }
            AgentSpec::Wasm(path) => build_wasm(path),
        },
        Profile::Hosted(limits) => {
            Ok(Box::new(ProcessRuntime::spawn(host_bin, spec, limits, timeout)?))
        }
    }
}

/// Everything needed to (re)build a managed agent.
#[derive(Clone, Debug)]
pub struct Recipe {
    pub spec: AgentSpec,
    pub profile: Profile,
    /// Path to the `agent-host` binary (used only by `Hosted`).
    pub host_bin: PathBuf,
    pub timeout: Duration,
    /// The agent's own UNL + DATA seed, replayed after every (re)start.
    pub seed_unl: Vec<u8>,
    pub seed_data: Vec<u8>,
    pub max_restarts: u32,
}

/// A live agent plus the recipe to rebuild it; restarts on failure.
pub struct ManagedAgent {
    runtime: Box<dyn AgentRuntime>,
    recipe: Recipe,
    restarts: u32,
}

impl ManagedAgent {
    /// Build, init, and seed the agent.
    pub fn spawn(recipe: Recipe) -> Result<Self> {
        let runtime = Self::start(&recipe)?;
        Ok(ManagedAgent { runtime, recipe, restarts: 0 })
    }

    fn start(recipe: &Recipe) -> Result<Box<dyn AgentRuntime>> {
        let mut rt = build_runtime(&recipe.spec, &recipe.profile, &recipe.host_bin, recipe.timeout)?;
        rt.init()?;
        let _ = rt.take_sends();
        if !recipe.seed_unl.is_empty() || !recipe.seed_data.is_empty() {
            rt.config("", &recipe.seed_unl, &recipe.seed_data)?; // seed: no sender
            let _ = rt.take_sends();
        }
        Ok(rt)
    }

    /// Deliver a message from `from`; on failure, restart (within budget) and
    /// surface the error so the caller can resend if it wishes.
    pub fn deliver(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<Vec<OutboundIntent>> {
        match self.runtime.config(from, unl, body) {
            Ok(()) => Ok(self.runtime.take_sends()),
            Err(e) => {
                self.restart()?;
                Err(e)
            }
        }
    }

    /// Rebuild the agent from its recipe, within the restart budget.
    pub fn restart(&mut self) -> Result<()> {
        if self.restarts >= self.recipe.max_restarts {
            return Err(anyhow!("agent exceeded max restarts ({})", self.recipe.max_restarts));
        }
        self.restarts += 1;
        self.runtime = Self::start(&self.recipe)?;
        Ok(())
    }

    pub fn restarts(&self) -> u32 {
        self.restarts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_process_factory_builds_a_native_agent() {
        let mut rt = build_runtime(
            &AgentSpec::Native("echo".into()),
            &Profile::InProcess,
            Path::new(""),
            Duration::ZERO,
        )
        .unwrap();
        rt.init().unwrap();
        rt.config("BA", b"agt(hi, x)", b"ping").unwrap();
        let sends = rt.take_sends();
        assert_eq!(sends[0].receiver, "peer");
        assert_eq!(sends[0].body, b"ping");
    }

    #[test]
    fn unknown_native_agent_is_an_error() {
        let r = build_runtime(
            &AgentSpec::Native("nope".into()),
            &Profile::InProcess,
            Path::new(""),
            Duration::ZERO,
        );
        assert!(r.is_err());
    }
}
