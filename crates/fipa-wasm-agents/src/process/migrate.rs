//! Agent migration — the move payload (`docs/MOBILITY.md`).
//!
//! Migration is **state-based** (engine-portable): a moving agent carries an
//! [`AgentSnapshot`] of its serialized state, **signed by the origin node**, not a
//! raw memory image. The destination verifies the signature before trusting a
//! byte, restores the state into the agent, and (R6) re-binds the agent's location
//! at AMS with a **higher epoch** so the move is the single forward step — a
//! replayed or forked snapshot at a lower/equal epoch cannot double-bind (H1).
//!
//! This module is the move payload + its signing. The orchestration (the two-phase
//! commit, the attestation chain for the destination's key, content-addressed code
//! transfer when the destination lacks the agent) is the remaining M5 hardening;
//! the happy-path state transfer + signed snapshot + epoch arbiter are implemented.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adapters::{self, NodeCrypto};

/// SHA-256 of `bytes` as lowercase hex — the content address of a wasm module.
pub fn code_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// A signed, state-based migration payload for one wasm agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// The migrating agent's instance UUID (unchanged across the move).
    pub uuid: String,
    /// The new location epoch (strictly greater than the agent's current epoch).
    pub epoch: u64,
    /// The agent's wasm module — may be **empty**, in which case the destination
    /// fetches it by `code_hash` (CODE_FETCH). Only wasm agents are mobile.
    pub code: Vec<u8>,
    /// Content address (SHA-256 hex) of the wasm module — always present, signed.
    pub code_hash: String,
    /// The agent's serialized state (from the guest's `snapshot` export).
    pub state: Vec<u8>,
    /// Anti-replay nonce.
    pub nonce: Vec<u8>,
    /// The origin node's Ed25519 public key.
    pub origin_pub: Vec<u8>,
    /// Signature by the origin node over everything above.
    pub sig: Vec<u8>,
}

impl AgentSnapshot {
    /// Build a snapshot signed by the origin node `key`. The signature covers the
    /// content `code_hash`, not the bytes — so an inlined or fetched module both
    /// verify against the same signed hash.
    pub fn sealed(uuid: &str, epoch: u64, code: Vec<u8>, state: Vec<u8>, key: &NodeCrypto) -> Self {
        let mut s = AgentSnapshot {
            uuid: uuid.into(),
            epoch,
            code_hash: code_hash(&code),
            code,
            state,
            nonce: key.nonce().to_vec(),
            origin_pub: key.public_key().to_vec(),
            sig: Vec::new(),
        };
        s.sig = key.sign(&s.signing_bytes()).to_vec();
        s
    }

    /// The bytes covered by the signature: every field except `sig` and the
    /// (fetchable) `code` — the `code_hash` stands in for the module.
    fn signing_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(self.uuid.as_bytes());
        b.push(0);
        b.extend_from_slice(&self.epoch.to_be_bytes());
        b.extend_from_slice(self.code_hash.as_bytes());
        b.extend_from_slice(&self.state);
        b.extend_from_slice(&self.nonce);
        b.extend_from_slice(&self.origin_pub);
        b
    }

    /// Verify the origin signature (integrity + origin authenticity), and that any
    /// **inlined** code matches its content address.
    pub fn verify(&self) -> bool {
        if self.sig.len() != 64 || self.origin_pub.len() != 32 {
            return false;
        }
        if !self.code.is_empty() && code_hash(&self.code) != self.code_hash {
            return false; // inlined module does not match its signed hash
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&self.origin_pub);
        let mut sg = [0u8; 64];
        sg.copy_from_slice(&self.sig);
        adapters::verify(&pk, &self.signing_bytes(), &sg)
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// A key handoff: the origin node authorizes a destination node to act for an
/// agent at a new epoch (`docs/MOBILITY.md` §7). The AMS node verifies this against
/// the agent's current TOFU key before moving the binding — so a legitimately
/// migrated agent can re-bind under the destination's key without breaking the R3
/// impersonation defense.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    pub agent: String,
    /// The current authorized node key (must match the agent's TOFU key at AMS).
    pub from_pub: Vec<u8>,
    /// The destination node key being authorized.
    pub to_pub: Vec<u8>,
    pub epoch: u64,
    /// Signature by `from_pub` (the origin node) over the fields above.
    pub sig: Vec<u8>,
}

impl Handoff {
    pub fn sealed(agent: &str, to_pub: Vec<u8>, epoch: u64, from_key: &NodeCrypto) -> Self {
        let mut h = Handoff {
            agent: agent.into(),
            from_pub: from_key.public_key().to_vec(),
            to_pub,
            epoch,
            sig: Vec::new(),
        };
        h.sig = from_key.sign(&h.signing_bytes()).to_vec();
        h
    }

    fn signing_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(self.agent.as_bytes());
        b.push(0);
        b.extend_from_slice(&self.from_pub);
        b.extend_from_slice(&self.to_pub);
        b.extend_from_slice(&self.epoch.to_be_bytes());
        b
    }

    /// Verify the handoff is signed by `from_pub`.
    pub fn verify(&self) -> bool {
        if self.sig.len() != 64 || self.from_pub.len() != 32 {
            return false;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&self.from_pub);
        let mut sg = [0u8; 64];
        sg.copy_from_slice(&self.sig);
        adapters::verify(&pk, &self.signing_bytes(), &sg)
    }
}

/// The full move payload sent over `KIND_MIGRATE`: a snapshot + the key handoff.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigratePayload {
    pub snapshot: AgentSnapshot,
    pub handoff: Handoff,
    /// The origin node's address, so the destination can CODE_FETCH a module that
    /// the snapshot left out.
    #[serde(default)]
    pub from_addr: String,
}

impl MigratePayload {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_sign_verify_roundtrip() {
        let k = NodeCrypto::generate();
        let snap = AgentSnapshot::sealed("CTR", 1, vec![0xaa, 0xbb], vec![0, 0, 0, 7], &k);
        assert!(snap.verify());
        let back = AgentSnapshot::decode(&snap.encode()).unwrap();
        assert!(back.verify());
        assert_eq!(back.code, vec![0xaa, 0xbb]);
        assert_eq!(back.state, vec![0, 0, 0, 7]);
        assert_eq!(back.epoch, 1);
    }

    #[test]
    fn tampered_snapshot_is_rejected() {
        let k = NodeCrypto::generate();
        let mut snap = AgentSnapshot::sealed("CTR", 1, vec![0xaa], vec![0, 0, 0, 7], &k);
        snap.state = vec![9, 9, 9, 9]; // tamper with the state after signing
        assert!(!snap.verify());
        let mut snap2 = AgentSnapshot::sealed("CTR", 1, vec![0xaa], vec![7], &k);
        snap2.code = vec![0xff]; // tamper with the code after signing
        assert!(!snap2.verify());
    }
}
