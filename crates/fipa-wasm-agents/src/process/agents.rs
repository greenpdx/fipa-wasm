//! Native agents compiled into the node / agent-host. The infrastructure agents
//! (DF, AMS, PA) will register here; for now there are two samples used by the
//! tests. Shared by the in-process factory and the agent-host child.

use crate::wasm::{AgentRuntime, NativeRuntime};
use unl_agent::{Agent, Ctx};

/// Echoes each message back to "peer".
struct Echo;
impl Agent for Echo {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        ctx.send("peer", unl, body.to_vec());
    }
}

/// Panics on a message containing "boom" (to exercise fault containment and
/// restart); otherwise echoes.
struct Boomer;
impl Agent for Boomer {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
        assert!(!unl.contains("boom"), "boom");
        ctx.send("peer", unl, body.to_vec());
    }
}

/// Build a native agent runtime by name; `None` for an unknown name.
pub fn native_agent(name: &str) -> Option<Box<dyn AgentRuntime>> {
    match name {
        "echo" => Some(Box::new(NativeRuntime::new(Echo))),
        "boomer" => Some(Box::new(NativeRuntime::new(Boomer))),
        _ => None,
    }
}
