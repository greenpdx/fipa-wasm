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

    /// Deliver `(unl, body)` to the agent — once at startup to seed it, then per
    /// inbound message.
    fn config(&mut self, unl: &[u8], body: &[u8]) -> Result<()>;

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
}

impl AgentRuntime for super::WasmRuntime {
    fn init(&mut self) -> Result<()> {
        self.call_init()
    }

    fn config(&mut self, unl: &[u8], body: &[u8]) -> Result<()> {
        self.call_config(unl, body)
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
}

/// Drives a native Rust [`Agent`] in-process. The same `Agent` impl that an
/// infrastructure agent (DF/AMS/PA) uses is wrapped here; a wasm agent uses
/// [`super::WasmRuntime`] instead — both look identical to the actor.
pub struct NativeRuntime<A: Agent> {
    agent: A,
    outbox: Vec<OutboundIntent>,
}

impl<A: Agent> NativeRuntime<A> {
    pub fn new(agent: A) -> Self {
        NativeRuntime { agent, outbox: Vec::new() }
    }

    fn drain(&mut self, ctx: &mut Ctx) {
        for out in ctx.take() {
            self.outbox.push(OutboundIntent {
                receiver: out.to,
                unl: out.unl.into_bytes(),
                body: out.body,
            });
        }
    }
}

impl<A: Agent> AgentRuntime for NativeRuntime<A> {
    fn init(&mut self) -> Result<()> {
        let mut ctx = Ctx::new();
        self.agent.on_init(&mut ctx);
        self.drain(&mut ctx);
        Ok(())
    }

    fn config(&mut self, unl: &[u8], body: &[u8]) -> Result<()> {
        // The vocabulary seed (UNL begins with '{') is not a message.
        if unl_agent::is_seed(unl) {
            return Ok(());
        }
        let text = std::str::from_utf8(unl).unwrap_or("");
        let mut ctx = Ctx::new();
        self.agent.on_message(text, body, &mut ctx);
        self.drain(&mut ctx);
        Ok(())
    }

    fn take_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut self.outbox)
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

    #[test]
    fn native_runtime_drives_an_agent() {
        let mut rt = NativeRuntime::new(Echoer);
        rt.init().unwrap();
        assert!(rt.take_sends().is_empty());

        // a seed config is ignored
        rt.config(b"{\"concepts\":{}}", b"peer").unwrap();
        assert!(rt.take_sends().is_empty());

        // a real message is handled
        rt.config(b"agt(greet, you)", b"hi").unwrap();
        let sends = rt.take_sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].receiver, "peer");
        assert_eq!(sends[0].unl, b"agt(greet, you)");
        assert_eq!(sends[0].body, b"hi");
    }
}
