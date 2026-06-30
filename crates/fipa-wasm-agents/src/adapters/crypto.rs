//! The node signing oracle (R1 rails; key custody per `AGENT_HOST_ABI.md` §7.2).
//!
//! A node holds an Ed25519 identity. The **secret never leaves this type** — the
//! rest of the node (and agents) only ever obtain `sign`/`verify` *results*, never
//! the key. Verification needs no secret, so it is a free function ([`verify`]).

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
    /// Mint a fresh node key from OS entropy.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        NodeCrypto { key: SigningKey::from_bytes(&seed) }
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
        restrict_perms(path); // M9: owner-only — a node secret must not be world-readable
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

/// Restrict a freshly-written secret-key file to owner read/write (`0o600`) so a
/// node secret persisted under a permissive umask is not left world-readable (M9).
fn restrict_perms(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Verify a detached Ed25519 signature. No secret needed — verification can happen
/// anywhere; only signing is node-side.
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
        let msg = b"obj(reserve, LtG)";
        let sig = k.sign(msg);
        assert!(verify(&k.public_key(), msg, &sig));
    }

    #[test]
    fn tamper_or_wrong_key_fails() {
        let k = NodeCrypto::generate();
        let other = NodeCrypto::generate();
        let sig = k.sign(b"hello");
        assert!(!verify(&k.public_key(), b"hell0", &sig)); // tampered message
        assert!(!verify(&other.public_key(), b"hello", &sig)); // wrong key
    }

    #[test]
    fn seed_persists_across_load() {
        let dir = std::env::temp_dir().join(format!("nodekey-{}", std::process::id()));
        let path = dir.join("node_key");
        let a = NodeCrypto::load_or_mint(&path).unwrap();
        let b = NodeCrypto::load_or_mint(&path).unwrap(); // "restart"
        assert_eq!(a.public_key(), b.public_key()); // stable identity
        std::fs::remove_dir_all(&dir).ok();
    }
}
