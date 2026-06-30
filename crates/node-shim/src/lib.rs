//! # node-shim — agent + node in one binary
//!
//! A microscopic FIPA node for constrained targets (ESP32 / esp-idf, std on
//! FreeRTOS). Instead of a node process that *hosts* a wasm agent, the agent is
//! native Rust compiled directly into the firmware and the **shim** is a library
//! that wraps it with everything the wire needs:
//!
//! - the signed message envelope ([`wire`], [`crypto`]) — R1,
//! - the Noise-encrypted channel ([`noise`]) — R2,
//! - a single-agent event loop ([`Shim`]) that gates inbound messages and routes
//!   the agent's replies.
//!
//! There is **no wasm engine** (wasmtime/wasmi/cranelift all drop out) and none of
//! the server stack (libp2p, tonic, sled, actix). The dependency surface is just
//! signing + Noise + hashing + entropy, so the agent and the protocols link into a
//! small image. The same [`Agent`] an embedded device runs is the same shape a
//! hosted wasm agent exposes — only the packaging differs.

pub mod agent;
pub mod crypto;
pub mod noise;
pub mod run;
pub mod wire;

pub use agent::{Agent, Ctx, Outgoing};
pub use crypto::{verify, NodeCrypto};
pub use noise::{NodeNoise, NoiseSession};
pub use run::Shim;
pub use wire::NodeMsg;
