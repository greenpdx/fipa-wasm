//! # node-core — shared FIPA node transport primitives
//!
//! The wire below the agent layer, factored out so the server-class node
//! ([`fipa-wasm-agents`]) and the embedded [`node-shim`] share one implementation
//! instead of two copies:
//!
//! - [`wire`] — the signed [`NodeMsg`] envelope + length-prefixed codec (R1),
//! - [`crypto`] — the node's Ed25519 identity ([`NodeCrypto`]) + [`verify`],
//! - [`noise`] — the Noise XX encrypted channel (R2) with static-key capture.
//!
//! No async runtime, no serde, no wasm engine — just signing, Noise, and entropy.
//! The *agent* abstraction is deliberately **not** here: the full node and the
//! embedded shim have different runtimes, and only these transport primitives are
//! genuinely common.

pub mod crypto;
pub mod noise;
mod resolver;
pub mod wire;

pub use crypto::{verify, NodeCrypto};
pub use noise::{NodeNoise, NoiseSession};
pub use wire::NodeMsg;
