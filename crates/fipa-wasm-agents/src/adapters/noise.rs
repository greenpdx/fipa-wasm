//! Authenticated, encrypted node transport (R2; `THREAT_MODEL.md` H2).
//!
//! The implementation now lives in the shared [`node_core`] crate so the full node
//! and the embedded node-shim share one Noise transport. Re-exported here to keep
//! the `adapters::{NodeNoise, NoiseSession}` paths stable.

pub use node_core::noise::{NodeNoise, NoiseSession};
