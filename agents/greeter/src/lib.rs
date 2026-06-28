//! The wasm template: a minimal agent built for wasm32 via `export_agent!`.
//!
//!   cargo build -p greeter-agent --target wasm32-unknown-unknown
//!
//! On a host build the `export_agent!` glue is `cfg(wasm32)`-compiled out, so
//! the type is unused — hence the allow below.
//!
//! `forbid(unsafe_code)` is the agent-crate policy: a safe-Rust agent cannot
//! corrupt node memory, the native counterpart of the wasm memory sandbox.
#![allow(dead_code)]
#![forbid(unsafe_code)]

use unl_agent::{Agent, Ctx};

/// Replies to whoever sent the message (uses the authenticated `ctx.from()`).
struct Greeter;

impl Agent for Greeter {
    fn on_message(&mut self, _unl: &str, _body: &[u8], ctx: &mut Ctx) {
        let from = ctx.from().to_string();
        ctx.send(from, "agt(greet, you)", b"hi from rust-wasm".to_vec());
    }
}

unl_agent::export_agent!(Greeter);
