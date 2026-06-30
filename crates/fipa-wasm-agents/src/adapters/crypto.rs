//! The node signing oracle (R1 rails; key custody per `AGENT_HOST_ABI.md` §7.2).
//!
//! The implementation now lives in the shared [`node_core`] crate so the full node
//! and the embedded node-shim share one signing oracle. Re-exported here to keep
//! the `adapters::{NodeCrypto, verify}` paths stable.

pub use node_core::crypto::{verify, NodeCrypto};
