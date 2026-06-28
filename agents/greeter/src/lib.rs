//! The wasm template: a minimal agent built for wasm32 via `export_agent!`.
//!
//!   cargo build -p greeter-agent --target wasm32-unknown-unknown
//!
//! On a host build the `export_agent!` glue is `cfg(wasm32)`-compiled out, so
//! the type is unused — hence the allow below.
#![allow(dead_code)]

use unl_agent::{Agent, Ctx};

/// Replies to any message by greeting its peer.
struct Greeter;

impl Agent for Greeter {
    fn on_message(&mut self, _unl: &str, _body: &[u8], ctx: &mut Ctx) {
        ctx.send("peer", "agt(greet, you)", b"hi from rust-wasm".to_vec());
    }
}

unl_agent::export_agent!(Greeter);
