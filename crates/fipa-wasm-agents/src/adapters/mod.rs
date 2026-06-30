//! Platform adapters — the swappable seams that let one node *kernel* run on the
//! normal / IoT / browser profiles (`NODE_DESIGN.md` §5).
//!
//! M1 introduces the seam traits up front and the concrete impls the current node
//! needs. `Engine` (wasm execution) and `LlmBackend` land with their own
//! milestones; here we provide [`Crypto`] (the node signing oracle), the reserved
//! sender policy ([`is_reserved_sender`], `THREAT_MODEL.md` C5), and the trait
//! shapes for [`Transport`], [`Clock`], and [`StateStore`].

pub mod crypto;
pub mod engine;
pub mod noise;
pub mod store;
pub use crypto::{verify, NodeCrypto};
pub use engine::{Engine, EngineModule, HostHooks, Limits, MAX_QUEUED_SENDS, MAX_SEND_BYTES};
pub use noise::{NodeNoise, NoiseSession};
pub use store::SledStore;

use anyhow::Result;

/// Reserved sender ids the node and agents trust *internally*. They MUST be
/// rejected if they ever arrive as a message `from` over the wire — otherwise a
/// remote attacker could inject forged discovery results, tool replies, or
/// kickoffs (`THREAT_MODEL.md` C5).
pub const RESERVED_SENDERS: &[&str] =
    &["ams", "df", "pa", "llm", "node", "crypto", "boot", "resolver", "result"];

/// True if `from` is a reserved system sender that must never originate on the wire.
pub fn is_reserved_sender(from: &str) -> bool {
    RESERVED_SENDERS.contains(&from)
}

/// Authenticated, length-bounded message transport between nodes (the FIPA ACC).
/// M1 ships a TCP impl in `process::node`; Noise/TLS channel auth (R2) and the
/// browser/IoT transports slot in here later.
pub trait Transport {
    /// Send one framed message to `addr`.
    fn send(&self, addr: &str, frame: &[u8]) -> Result<()>;
}

/// The node signing oracle — keys stay node-side (`AGENT_HOST_ABI.md` §7.2).
/// `verify` is the free function [`verify`] (no secret needed).
pub trait Crypto {
    fn public_key(&self) -> [u8; 32];
    fn sign(&self, msg: &[u8]) -> [u8; 64];
    fn nonce(&self) -> [u8; 16];
}

impl Crypto for NodeCrypto {
    fn public_key(&self) -> [u8; 32] {
        NodeCrypto::public_key(self)
    }
    fn sign(&self, msg: &[u8]) -> [u8; 64] {
        NodeCrypto::sign(self, msg)
    }
    fn nonce(&self) -> [u8; 16] {
        NodeCrypto::nonce(self)
    }
}

/// Wall + monotonic clock and timer source (drives `time`/`tick`, ABI §9).
pub trait Clock {
    fn now_ms(&self) -> u64;
    fn mono_ns(&self) -> u128;
}

/// Agent-scoped durable key-value store; keys are confined to the agent's
/// namespace (no escape — `THREAT_MODEL.md` R8).
pub trait StateStore {
    fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>>;
    fn put(&self, ns: &str, key: &str, val: &[u8]) -> Result<()>;
    fn del(&self, ns: &str, key: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_senders_are_flagged() {
        assert!(is_reserved_sender("ams"));
        assert!(is_reserved_sender("boot"));
        assert!(is_reserved_sender("llm"));
        assert!(!is_reserved_sender("7f3a9c2e")); // an ordinary agent uuid
    }
}
