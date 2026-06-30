//! The agent contract. The same shape a hosted wasm agent exposes, but here the
//! agent is native Rust compiled into the firmware.

/// One message the agent wants to send. `unl` is the semantic content (UNL text),
/// `body` an opaque payload.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub to: String,
    pub unl: String,
    pub body: Vec<u8>,
}

/// The per-message context handed to the agent: who the (authenticated) sender is,
/// and an outbox the shim drains after the call.
#[derive(Default)]
pub struct Ctx {
    from: String,
    out: Vec<Outgoing>,
}

impl Ctx {
    pub fn new(from: &str) -> Self {
        Ctx { from: from.into(), out: Vec::new() }
    }

    /// The authenticated sender of the message being handled.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Queue a reply / outbound message.
    pub fn send(&mut self, to: impl Into<String>, unl: impl Into<String>, body: Vec<u8>) {
        self.out.push(Outgoing { to: to.into(), unl: unl.into(), body });
    }

    /// Drain what the agent emitted (the shim routes these).
    pub fn take(self) -> Vec<Outgoing> {
        self.out
    }
}

/// A device agent. One method: react to a delivered message by emitting replies.
pub trait Agent {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx);
}
