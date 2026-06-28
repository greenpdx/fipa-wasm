//! Native agent registry. Sample agents (echo/boomer) used by the tests live
//! here; the infrastructure agents (DF, AMS, PA) live in their own crates under
//! `agents/` and register below once built.

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
        "df" => Some(Box::new(NativeRuntime::new(df_agent::Df::new()))),
        "ams" => Some(Box::new(NativeRuntime::new(ams_agent::Ams::new()))),
        _ => None,
    }
}
