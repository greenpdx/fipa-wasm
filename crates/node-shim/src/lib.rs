//! # node-shim — agent + node in one binary
//!
//! A microscopic FIPA node for constrained targets (ESP32 / esp-idf, std on
//! FreeRTOS). Instead of a node process that *hosts* a wasm agent, the agent is
//! native Rust compiled directly into the firmware and the **shim** wraps it with a
//! single-agent event loop ([`Shim`]).
//!
//! The wire primitives — the signed envelope, Ed25519 identity, and the Noise
//! channel — come from the shared [`node_core`] crate (the same code the full node
//! uses), so a shim device and a hosted node interoperate by construction. There is
//! **no wasm engine** and none of the server stack; the shim adds only the embedded
//! [`Agent`] contract and the loop that gates, delivers, and routes its messages.

pub mod agent;
pub mod run;

pub use agent::{Agent, Ctx, Outgoing};
pub use run::Shim;

// One import root for a device crate: the shared transport primitives + the shim.
pub use node_core::{crypto, noise, verify, wire, NodeCrypto, NodeMsg, NodeNoise, NoiseSession};
