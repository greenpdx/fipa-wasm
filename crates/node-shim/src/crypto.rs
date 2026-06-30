//! The node's Ed25519 identity. The secret never leaves this type; the rest of the
//! firmware only obtains sign/verify results.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;

/// A node's Ed25519 keypair. Secret-side only.
#[derive(Clone)]
pub struct NodeCrypto {
    key: SigningKey,
}

impl NodeCrypto {
    /// Mint a fresh node key from OS/hardware entropy.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        NodeCrypto { key: SigningKey::from_bytes(&seed) }
    }

    /// Reconstruct from a persisted 32-byte secret seed (e.g. from NVS flash).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        NodeCrypto { key: SigningKey::from_bytes(seed) }
    }

    /// The 32-byte secret seed, to persist in secure storage.
    pub fn seed(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    /// This node's public key (32 bytes) — safe to share.
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Detached signature over `msg` (64 bytes).
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.key.sign(msg).to_bytes()
    }

    /// A fresh 16-byte anti-replay nonce.
    pub fn nonce(&self) -> [u8; 16] {
        let mut n = [0u8; 16];
        rand::rng().fill_bytes(&mut n);
        n
    }
}

/// Verify a detached Ed25519 signature. No secret needed.
pub fn verify(pubkey: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else { return false };
    vk.verify_strict(msg, &Signature::from_bytes(sig)).is_ok()
}
