//! The wasm execution-backend seam (`NODE_DESIGN.md` §5).
//!
//! The node drives every agent through `AgentRuntime`; a *wasm* agent's
//! `AgentRuntime` is implemented on top of an [`Engine`]. Today the only engine is
//! wasmtime ([`crate::wasm::WasmRuntime`]); this trait is the seam an alternate
//! backend implements so the **same agent ABI** runs on a constrained profile:
//!
//! - **wasmi** — a small interpreter for the **IoT** profile (no JIT);
//! - **browser-WASM** — the host's own engine via JS, for the **browser** profile.
//!
//! Status: this is the **defined seam**, not yet adopted — `WasmRuntime` remains a
//! concrete wasmtime implementation. The porting plan (E1 refactor → E2 wasmi → E3
//! browser) is in the module docs below.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::wasm::OutboundIntent;

/// Resource limits an engine enforces per agent (H3/R7).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Per-call CPU budget (fuel / gas).
    pub fuel: u64,
    /// Linear-memory ceiling in bytes.
    pub mem_bytes: usize,
}

/// The host import table wired into a module. The `send-unl` import appends to
/// `sends`, which the node drains after each call (the agent's only ambient
/// authority — every other capability is a separate, gated host call).
#[derive(Clone, Default)]
pub struct HostHooks {
    pub sends: Arc<Mutex<Vec<OutboundIntent>>>,
}

/// A wasm execution backend: compiles + instantiates a module with the node's host
/// imports and resource limits. wasmtime is the reference impl; wasmi (IoT) and
/// browser-WASM are the porting targets. One trait, swapped per node profile.
pub trait Engine: Send {
    /// An instantiated agent module (its store, memory, and host state).
    type Module: EngineModule;

    /// Compile `code` and instantiate it under `limits` with the host import table.
    fn instantiate(&self, code: &[u8], limits: Limits, hooks: HostHooks) -> Result<Self::Module>;
}

/// An instantiated module — the typed entry points the wasm `AgentRuntime` drives.
/// The agent ABI maps on as: `init`/`shutdown` → [`call_void`], `run` → [`call_i32`],
/// `config`/`deliver`/`restore` → [`call_io`], `snapshot` → [`call_packed`].
///
/// [`call_void`]: EngineModule::call_void
/// [`call_i32`]: EngineModule::call_i32
/// [`call_io`]: EngineModule::call_io
/// [`call_packed`]: EngineModule::call_packed
pub trait EngineModule: Send {
    /// Reset the per-call fuel budget before an entry point (the CPU cap, H3/R7).
    fn refuel(&mut self, fuel: u64);

    /// Call an exported `() -> ()` function (`init`, `shutdown`).
    fn call_void(&mut self, func: &str) -> Result<()>;

    /// Call an exported `() -> i32` function (`run`); `Ok(false)` if absent.
    fn call_i32(&mut self, func: &str) -> Result<i32>;

    /// Call an export taking a flat list of `(ptr, len)` byte slices: the host
    /// `alloc`s + writes each slice into guest memory and passes the pairs
    /// (`config`, `deliver`, `restore`). `Ok(false)` if the export is absent.
    fn call_io(&mut self, func: &str, args: &[&[u8]]) -> Result<bool>;

    /// Call an export returning a packed `(ptr << 32) | len` and read those bytes
    /// from guest memory (`snapshot`). Empty if the export is absent.
    fn call_packed(&mut self, func: &str) -> Result<Vec<u8>>;
}
