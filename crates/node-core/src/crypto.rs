//! The node signing oracle (R1 rails). A node holds an Ed25519 identity; the
//! **secret never leaves this type** — callers only obtain `sign`/`verify` results.
//! Verification needs no secret, so it is a free function ([`verify`]).

use std::fs;
use std::io;
use std::path::Path;

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

    /// Reconstruct from a persisted 32-byte secret seed (e.g. embedded NVS flash).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        NodeCrypto { key: SigningKey::from_bytes(seed) }
    }

    /// The 32-byte secret seed, to persist in secure storage.
    pub fn seed(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    /// Load the persisted 32-byte secret seed at `path`, or mint + persist one.
    pub fn load_or_mint(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Ok(seed) = fs::read(path) {
            if seed.len() == 32 {
                let mut s = [0u8; 32];
                s.copy_from_slice(&seed);
                return Ok(NodeCrypto { key: SigningKey::from_bytes(&s) });
            }
        }
        let me = Self::generate();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, me.key.to_bytes())?; // the 32-byte secret seed
        restrict_perms(path); // owner-only — a node secret must not be world-readable
        Ok(me)
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

/// Restrict a freshly-written secret-key file to owner read/write (`0o600`).
fn restrict_perms(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Verify a detached Ed25519 signature. No secret needed.
pub fn verify(pubkey: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else { return false };
    vk.verify_strict(msg, &Signature::from_bytes(sig)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let k = NodeCrypto::generate();
        let sig = k.sign(b"obj(reserve, LtG)");
        assert!(verify(&k.public_key(), b"obj(reserve, LtG)", &sig));
    }

    #[test]
    fn tamper_or_wrong_key_fails() {
        let k = NodeCrypto::generate();
        let other = NodeCrypto::generate();
        let sig = k.sign(b"hello");
        assert!(!verify(&k.public_key(), b"hell0", &sig));
        assert!(!verify(&other.public_key(), b"hello", &sig));
    }

    #[test]
    fn seed_roundtrips() {
        let k = NodeCrypto::generate();
        let k2 = NodeCrypto::from_seed(&k.seed());
        assert_eq!(k.public_key(), k2.public_key());
    }
}
