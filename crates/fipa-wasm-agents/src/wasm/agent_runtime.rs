//! The `AgentRuntime` seam — what the node drives, regardless of substrate.
//!
//! An agent is either a sandboxed wasm module ([`super::WasmRuntime`]) or a
//! native Rust [`Agent`] running in-process ([`NativeRuntime`]). Both present
//! the same lifecycle to the actor: `init` once, `config(unl, body)` to seed
//! then per message, and `take_sends` to collect what the agent emitted.

use anyhow::Result;
use unl_agent::{Agent, Ctx};

use super::host::OutboundIntent;

/// What the actor drives. Implemented by the wasm runtime and by the native
/// in-process runtime.
pub trait AgentRuntime {
    /// Run the agent's `init` entry point.
    fn init(&mut self) -> Result<()>;

    /// Deliver `(unl, body)` to the agent from sender `from` — once at startup
    /// to seed it (`from == ""`), then per inbound message.
    fn config(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<()>;

    /// Drain the messages the agent emitted (validated + packaged by the node).
    fn take_sends(&mut self) -> Vec<OutboundIntent>;

    /// Per-tick run hook. Default: keep running.
    fn run(&mut self) -> Result<bool> {
        Ok(true)
    }

    /// Shutdown hook. Default: nothing.
    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }

    /// Capture the agent's state for migration (default: empty / stateless).
    fn snapshot(&mut self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore migrated state captured by [`AgentRuntime::snapshot`] (default: ignore).
    fn restore(&mut self, _state: &[u8]) {}

    /// Fire a scheduled timer tick into the agent (default: no-op).
    fn tick(&mut self, _timer_id: u64, _now_ms: u64) -> Result<()> {
        Ok(())
    }

    /// Drain the timer requests the agent made this call (default: none).
    fn take_timer_ops(&mut self) -> Vec<unl_agent::TimerOp> {
        Vec::new()
    }

    /// Provision the agent's durable-state handle (the `state` capability; default:
    /// ignore). The node calls this at mount only when `State` is granted.
    fn set_state(&mut self, _kv: std::sync::Arc<dyn unl_agent::Kv>) {}

    /// Provision the agent's crypto keyring (the `crypto` capability; default: ignore).
    fn set_keyring(&mut self, _kr: std::sync::Arc<dyn unl_agent::Keyring>) {}

    /// Drain the agent's inference requests this call (the `llm` capability; default:
    /// none). Each is run by the host and replied to as a message from `"llm"`.
    fn take_infer_reqs(&mut self) -> Vec<unl_agent::InferReq> {
        Vec::new()
    }

    /// Drain the agent's spawn requests this call (the `spawn` capability; default: none).
    fn take_spawn_reqs(&mut self) -> Vec<unl_agent::SpawnReq> {
        Vec::new()
    }
}

impl AgentRuntime for super::WasmRuntime {
    fn init(&mut self) -> Result<()> {
        self.call_init()
    }

    fn config(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<()> {
        // Prefer the from-aware `deliver` export; fall back to `config` for
        // guests that don't export it (hand-written WAT agents).
        if self.call_deliver(from.as_bytes(), unl, body)? {
            Ok(())
        } else {
            self.call_config(unl, body)
        }
    }

    fn take_sends(&mut self) -> Vec<OutboundIntent> {
        self.take_unl_sends()
    }

    fn run(&mut self) -> Result<bool> {
        self.call_run()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.call_shutdown()
    }

    fn snapshot(&mut self) -> Vec<u8> {
        self.call_snapshot()
    }

    fn restore(&mut self, state: &[u8]) {
        self.call_restore(state);
    }
}

/// Drives a native Rust [`Agent`] in-process. The same `Agent` impl that an
/// infrastructure agent (DF/AMS/PA) uses is wrapped here; a wasm agent uses
/// [`super::WasmRuntime`] instead — both look identical to the actor.
pub struct NativeRuntime<A: Agent> {
    agent: A,
    outbox: Vec<OutboundIntent>,
    timer_ops: Vec<unl_agent::TimerOp>,
    state: Option<std::sync::Arc<dyn unl_agent::Kv>>,
    keyring: Option<std::sync::Arc<dyn unl_agent::Keyring>>,
    infers: Vec<unl_agent::InferReq>,
    spawns: Vec<unl_agent::SpawnReq>,
}

impl<A: Agent> NativeRuntime<A> {
    pub fn new(agent: A) -> Self {
        NativeRuntime {
            agent,
            outbox: Vec::new(),
            timer_ops: Vec::new(),
            state: None,
            keyring: None,
            infers: Vec::new(),
            spawns: Vec::new(),
        }
    }

    /// Run one agent call with **fault isolation**: a panic is caught so it can
    /// never unwind into the node. On panic the agent's emitted output is
    /// discarded (its state is suspect) and the call fails, so the supervisor
    /// can quarantine or restart it. (Requires `panic = "unwind"`; with
    /// `panic = "abort"` only a process boundary contains a faulting agent.)
    fn guarded(&mut self, call: impl FnOnce(&mut A, &mut Ctx)) -> Result<()> {
        let kv = self.state.clone();
        let kr = self.keyring.clone();
        let agent = &mut self.agent;
        let mut ctx = Ctx::new();
        if let Some(s) = kv {
            ctx.set_state(s);
        }
        if let Some(k) = kr {
            ctx.set_keyring(k);
        }
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(agent, &mut ctx)));
        match outcome {
            Ok(()) => {
                for out in ctx.take() {
                    self.outbox.push(OutboundIntent {
                        receiver: out.to,
                        unl: out.unl.into_bytes(),
                        body: out.body,
                    });
                }
                self.timer_ops.extend(ctx.take_timers());
                self.infers.extend(ctx.take_infers());
                self.spawns.extend(ctx.take_spawns());
                Ok(())
            }
            Err(_) => Err(anyhow::anyhow!("native agent panicked; output discarded")),
        }
    }
}

impl<A: Agent> AgentRuntime for NativeRuntime<A> {
    fn init(&mut self) -> Result<()> {
        self.guarded(|a, ctx| a.on_init(ctx))
    }

    fn config(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<()> {
        // The seed (UNL begins with '{') carries the agent's DATA block.
        if unl_agent::is_seed(unl) {
            return self.guarded(move |a, ctx| a.on_seed(body, ctx));
        }
        let text = std::str::from_utf8(unl).unwrap_or("").to_string();
        let from = from.to_string();
        self.guarded(move |a, ctx| {
            ctx.set_from(&from);
            a.on_message(&text, body, ctx);
        })
    }

    fn take_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut self.outbox)
    }

    fn snapshot(&mut self) -> Vec<u8> {
        self.agent.snapshot()
    }

    fn restore(&mut self, state: &[u8]) {
        self.agent.restore(state);
    }

    fn tick(&mut self, timer_id: u64, now_ms: u64) -> Result<()> {
        self.guarded(|a, ctx| a.on_tick(timer_id, now_ms, ctx))
    }

    fn take_timer_ops(&mut self) -> Vec<unl_agent::TimerOp> {
        std::mem::take(&mut self.timer_ops)
    }

    fn set_state(&mut self, kv: std::sync::Arc<dyn unl_agent::Kv>) {
        self.state = Some(kv);
    }

    fn set_keyring(&mut self, kr: std::sync::Arc<dyn unl_agent::Keyring>) {
        self.keyring = Some(kr);
    }

    fn take_infer_reqs(&mut self) -> Vec<unl_agent::InferReq> {
        std::mem::take(&mut self.infers)
    }

    fn take_spawn_reqs(&mut self) -> Vec<unl_agent::SpawnReq> {
        std::mem::take(&mut self.spawns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echoer;
    impl Agent for Echoer {
        fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
            // bounce the message back to "peer"
            ctx.send("peer", unl, body.to_vec());
        }
    }

    struct Panicker;
    impl Agent for Panicker {
        fn on_message(&mut self, _unl: &str, _body: &[u8], _ctx: &mut Ctx) {
            panic!("agent went rogue");
        }
    }

    #[test]
    fn panicking_agent_is_contained() {
        let mut rt = NativeRuntime::new(Panicker);
        // The panic is caught; the node survives and no output leaks.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // silence the panic print
        let r = rt.config("alice", b"agt(x, y)", b"");
        std::panic::set_hook(prev);
        assert!(r.is_err());
        assert!(rt.take_sends().is_empty());
    }

    #[test]
    fn native_runtime_drives_an_agent() {
        let mut rt = NativeRuntime::new(Echoer);
        rt.init().unwrap();
        assert!(rt.take_sends().is_empty());

        // a seed config is ignored
        rt.config("", b"{\"concepts\":{}}", b"peer").unwrap();
        assert!(rt.take_sends().is_empty());

        // a real message is handled
        rt.config("alice", b"agt(greet, you)", b"hi").unwrap();
        let sends = rt.take_sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].receiver, "peer");
        assert_eq!(sends[0].unl, b"agt(greet, you)");
        assert_eq!(sends[0].body, b"hi");
    }
}
