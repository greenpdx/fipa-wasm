//! Author-facing agent API — **one trait, two targets**.
//!
//! An agent is written once against [`Agent`]. The *same code* can be built:
//! - **native** (`rlib`): linked into the node and driven in-process — used for
//!   the well-defined, stationary infrastructure agents (DF, AMS, PA);
//! - **wasm32** (`cdylib`): sandboxed and mobile — used for BA (and optionally
//!   BS). [`export_agent!`] wires the agent to the host ABI
//!   (`init`/`run`/`alloc`/`config` exports + the `send-unl` import).
//!
//! The agent never touches the ABI: it reacts to messages and emits replies
//! through [`Ctx`]. The host driver (native `NativeRuntime` or the wasm glue)
//! collects those replies — so both substrates behave identically.
//!
//! ## Isolation
//!
//! A native agent must be sandboxed *like* a wasm one so a corrupt agent cannot
//! poison the node. The four wasm guarantees map to native as:
//! - **capability** — the agent only gets `&mut self` + [`Ctx`]; no node handles
//!   (it cannot reach the supervisor, network, or other agents);
//! - **memory** — agent crates set `#![forbid(unsafe_code)]`, so safe Rust
//!   cannot corrupt host memory;
//! - **fault** — `NativeRuntime` runs every call under `catch_unwind`, so a
//!   panic is contained and the agent is quarantined, not the node;
//! - **resource** (hard CPU/RAM caps) — *not* achievable in-process; needs a
//!   thread or process boundary. That is the remaining isolation upgrade.
//!
//! ```ignore
//! struct Greeter { peer: String }
//! impl unl_agent::Agent for Greeter {
//!     fn on_message(&mut self, unl: &str, _body: &[u8], ctx: &mut unl_agent::Ctx) {
//!         ctx.send(&self.peer, "agt(greet, you)", b"hi");
//!     }
//! }
//! // wasm32 build only — exports the ABI:
//! unl_agent::export_agent!(Greeter { peer: "bob".into() });
//! ```

/// A message an agent wants to send: the receiver, the UNL content, and the
/// data payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub to: String,
    pub unl: String,
    pub body: Vec<u8>,
}

/// A namespaced, durable key-value handle the host grants to an agent that holds
/// the `state` capability. Reads/writes are **synchronous** and confined to the
/// agent's own namespace by the host (the agent cannot escape it). An agent without
/// the capability simply has no handle, so reads return `None` and writes are no-ops
/// (the uniform denial).
pub trait Kv: Send + Sync {
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn put(&self, key: &str, val: &[u8]);
    fn del(&self, key: &str);
}

/// A timer request an agent makes via [`Ctx::set_timer`] / [`Ctx::cancel_timer`].
/// The host schedules it (subject to the `Time` grant + slot budget) and later
/// calls [`Agent::on_tick`] when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerOp {
    Set { id: u64, delay_ms: u64 },
    Cancel { id: u64 },
}

/// The per-call context handed to an agent: the sender of the current message,
/// and a sink for outgoing replies. The host driver sets the sender and drains
/// the replies, so the agent is oblivious to whether it runs native or in wasm.
#[derive(Default)]
pub struct Ctx {
    from: String,
    sends: Vec<Outgoing>,
    timers: Vec<TimerOp>,
    state: Option<std::sync::Arc<dyn Kv>>,
}

impl Ctx {
    pub fn new() -> Self {
        Self::default()
    }

    /// The id of the agent that sent the current message (`""` if unknown, e.g.
    /// during the seed). Reply with `ctx.send(ctx.from().to_string(), ...)`.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Set the current sender — called by the runtime before delivering.
    pub fn set_from(&mut self, from: &str) {
        self.from.clear();
        self.from.push_str(from);
    }

    /// Send a message to another agent (by id).
    pub fn send(&mut self, to: impl Into<String>, unl: impl Into<String>, body: impl Into<Vec<u8>>) {
        self.sends.push(Outgoing { to: to.into(), unl: unl.into(), body: body.into() });
    }

    /// Drain the messages emitted during this call.
    pub fn take(&mut self) -> Vec<Outgoing> {
        core::mem::take(&mut self.sends)
    }

    /// Arm timer `id` to fire after `delay_ms` (the host then calls
    /// [`Agent::on_tick`]). Requires the `Time` capability; over the slot budget
    /// the host silently drops it (the agent sees a uniform denial).
    pub fn set_timer(&mut self, id: u64, delay_ms: u64) {
        self.timers.push(TimerOp::Set { id, delay_ms });
    }

    /// Cancel timer `id`.
    pub fn cancel_timer(&mut self, id: u64) {
        self.timers.push(TimerOp::Cancel { id });
    }

    /// Drain the timer requests emitted during this call (host-internal).
    pub fn take_timers(&mut self) -> Vec<TimerOp> {
        core::mem::take(&mut self.timers)
    }

    /// Install the agent's durable-state handle (host-internal; set before delivery
    /// only when the agent holds the `state` capability).
    pub fn set_state(&mut self, kv: std::sync::Arc<dyn Kv>) {
        self.state = Some(kv);
    }

    /// Read durable state (`None` without the `state` capability, or if absent).
    pub fn state_get(&self, key: &str) -> Option<Vec<u8>> {
        self.state.as_ref()?.get(key)
    }

    /// Write durable state (a no-op without the `state` capability).
    pub fn state_put(&self, key: &str, val: &[u8]) {
        if let Some(s) = &self.state {
            s.put(key, val);
        }
    }

    /// Delete durable state (a no-op without the `state` capability).
    pub fn state_del(&self, key: &str) {
        if let Some(s) = &self.state {
            s.del(key);
        }
    }
}

/// An agent. The same trait whether the agent is compiled native or to wasm32.
pub trait Agent {
    /// Called once before any message, with no data. Use for pure setup.
    fn on_init(&mut self, _ctx: &mut Ctx) {}

    /// Called once at startup with the agent's own `DATA` seed block (e.g. an
    /// infrastructure agent's initial registry). Default: ignore.
    fn on_seed(&mut self, _data: &[u8], _ctx: &mut Ctx) {}

    /// Called per inbound message with the decoded UNL text and the data
    /// payload; the sender is `ctx.from()`. Reply via `ctx.send(...)`.
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx);

    /// Serialize the agent's durable state for **migration** (default: stateless,
    /// so a stateless agent migrates trivially). State-based mobility carries this
    /// blob, not raw memory, so it is engine-portable (see `docs/MOBILITY.md`).
    fn snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore state previously captured by [`Agent::snapshot`] (default: ignore).
    fn restore(&mut self, _state: &[u8]) {}

    /// Called when a timer armed via [`Ctx::set_timer`] fires (default: ignore).
    /// `now_ms` is wall-clock milliseconds; reply or re-arm via `ctx`. This is what
    /// makes an agent **autonomous** — it can act without an inbound message.
    fn on_tick(&mut self, _timer_id: u64, _now_ms: u64, _ctx: &mut Ctx) {}
}

// ─────────────────────────────  wasm32 glue  ─────────────────────────────
// The host↔guest ABI, active only when the agent is built for wasm32. Native
// builds drop all of this (the node provides the driver instead).

/// `true` if `unl` is the vocabulary seed (a JSON object) rather than a message.
pub fn is_seed(unl: &[u8]) -> bool {
    unl.first() == Some(&b'{')
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod wasm_glue {
    use super::Outgoing;

    #[link(wasm_import_module = "fipa:agent/messaging")]
    unsafe extern "C" {
        #[link_name = "send-unl"]
        unsafe fn host_send_unl(
            rp: *const u8,
            rl: usize,
            up: *const u8,
            ul: usize,
            bp: *const u8,
            bl: usize,
        );
    }

    /// Emit one outgoing message to the host.
    pub fn emit(out: &Outgoing) {
        unsafe {
            host_send_unl(
                out.to.as_ptr(),
                out.to.len(),
                out.unl.as_ptr(),
                out.unl.len(),
                out.body.as_ptr(),
                out.body.len(),
            );
        }
    }

    /// The host calls this to reserve `len` bytes before writing an inbound
    /// `(unl, body)` and calling `config`.
    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: usize) -> *mut u8 {
        let mut v = Vec::<u8>::with_capacity(len.max(1));
        let p = v.as_mut_ptr();
        core::mem::forget(v);
        p
    }
}

/// Wire an [`Agent`] to the host ABI (wasm32 only). Defines the `init`, `run`,
/// `config`, and `alloc` exports that drive a single agent instance, decoding
/// inbound `(unl, body)` and forwarding the agent's replies to the host. The
/// vocabulary seed (UNL beginning with `{`) is skipped.
#[macro_export]
macro_rules! export_agent {
    ($init:expr) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            static mut AGENT: ::core::option::Option<::std::boxed::Box<dyn $crate::Agent>> = None;

            fn drive<F: FnOnce(&mut dyn $crate::Agent, &mut $crate::Ctx)>(f: F) {
                // wasm32 is single-threaded: exclusive access is sound.
                let agent = unsafe {
                    let slot = ::core::ptr::addr_of_mut!(AGENT);
                    if (*slot).is_none() {
                        *slot = ::core::option::Option::Some(::std::boxed::Box::new($init));
                    }
                    (*slot).as_mut().unwrap().as_mut()
                };
                let mut ctx = $crate::Ctx::new();
                f(agent, &mut ctx);
                for out in ctx.take() {
                    $crate::wasm_glue::emit(&out);
                }
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn init() {
                drive(|a, ctx| a.on_init(ctx));
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn run() -> i32 {
                1 // keep running
            }

            fn handle(from: &[u8], unl: &[u8], body: &[u8]) {
                let from = ::core::str::from_utf8(from).unwrap_or("");
                if $crate::is_seed(unl) {
                    drive(|a, ctx| {
                        ctx.set_from(from);
                        a.on_seed(body, ctx); // DATA seed → on_seed
                    });
                } else {
                    let unl = ::core::str::from_utf8(unl).unwrap_or("");
                    drive(|a, ctx| {
                        ctx.set_from(from);
                        a.on_message(unl, body, ctx);
                    });
                }
            }

            // No-sender delivery (back-compat: the host's `call_config`).
            #[unsafe(no_mangle)]
            pub extern "C" fn config(up: *const u8, ul: usize, bp: *const u8, bl: usize) {
                let unl = unsafe { ::core::slice::from_raw_parts(up, ul) };
                let body = unsafe { ::core::slice::from_raw_parts(bp, bl) };
                handle(&[], unl, body);
            }

            // From-aware delivery: the sender id is the first (ptr,len) pair.
            #[unsafe(no_mangle)]
            pub extern "C" fn deliver(
                fp: *const u8,
                fl: usize,
                up: *const u8,
                ul: usize,
                bp: *const u8,
                bl: usize,
            ) {
                let from = unsafe { ::core::slice::from_raw_parts(fp, fl) };
                let unl = unsafe { ::core::slice::from_raw_parts(up, ul) };
                let body = unsafe { ::core::slice::from_raw_parts(bp, bl) };
                handle(from, unl, body);
            }

            // State-based migration: `snapshot` returns the agent's serialized
            // state as a packed (ptr<<32 | len); `restore` re-applies it. Only wasm
            // agents are mobile, so this is the move payload's state half.
            #[unsafe(no_mangle)]
            pub extern "C" fn snapshot() -> i64 {
                let bytes = unsafe {
                    let slot = ::core::ptr::addr_of_mut!(AGENT);
                    if (*slot).is_none() {
                        *slot = ::core::option::Option::Some(::std::boxed::Box::new($init));
                    }
                    (*slot).as_ref().unwrap().snapshot()
                };
                let len = bytes.len() as i64;
                let ptr = bytes.as_ptr() as i64;
                ::core::mem::forget(bytes); // host reads it, then the instance is torn down
                (ptr << 32) | len
            }

            #[unsafe(no_mangle)]
            pub extern "C" fn restore(p: *const u8, l: usize) {
                let state = unsafe { ::core::slice::from_raw_parts(p, l) };
                drive(|a, _ctx| a.restore(state));
            }

            // re-export the host allocator so the linker keeps it
            pub use $crate::wasm_glue::alloc;
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Greeter {
        peer: String,
    }
    impl Agent for Greeter {
        fn on_message(&mut self, _unl: &str, _body: &[u8], ctx: &mut Ctx) {
            ctx.send(&self.peer, "agt(greet, you)", b"hi".to_vec());
        }
    }

    #[test]
    fn agent_emits_via_ctx() {
        let mut g = Greeter { peer: "bob".into() };
        let mut ctx = Ctx::new();
        g.on_message("agt(hello, me)", b"", &mut ctx);
        let out = ctx.take();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to, "bob");
        assert_eq!(out[0].unl, "agt(greet, you)");
        assert_eq!(out[0].body, b"hi");
    }

    #[test]
    fn seed_detection() {
        assert!(is_seed(b"{\"concepts\":{}}"));
        assert!(!is_seed(b"agt(greet, you)"));
    }
}
