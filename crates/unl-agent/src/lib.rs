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

/// The per-call context handed to an agent: the sender of the current message,
/// and a sink for outgoing replies. The host driver sets the sender and drains
/// the replies, so the agent is oblivious to whether it runs native or in wasm.
#[derive(Default)]
pub struct Ctx {
    from: String,
    sends: Vec<Outgoing>,
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

            #[unsafe(no_mangle)]
            pub extern "C" fn config(up: *const u8, ul: usize, bp: *const u8, bl: usize) {
                let unl = unsafe { ::core::slice::from_raw_parts(up, ul) };
                let body = unsafe { ::core::slice::from_raw_parts(bp, bl) };
                if $crate::is_seed(unl) {
                    drive(|a, ctx| a.on_seed(body, ctx)); // DATA seed → on_seed
                    return;
                }
                let unl = ::core::str::from_utf8(unl).unwrap_or("");
                drive(|a, ctx| a.on_message(unl, body, ctx));
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
