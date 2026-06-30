//! A minimal Noise crypto resolver for exactly the suite we use:
//! `Noise_XX_25519_ChaChaPoly_BLAKE2s`. Built so `snow` can be pulled with
//! `default-features = false` — its `default-resolver` otherwise bundles AES-GCM and
//! SHA-2, which we never use, as dead weight in flash.
//!
//! The primitive impls below mirror `snow`'s own `DefaultResolver` (same crypto
//! crates: `chacha20poly1305`, `blake2`, `curve25519-dalek`), so the wire bytes are
//! identical and a node-core peer interoperates with a stock-snow peer.

use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use curve25519_dalek::montgomery::MontgomeryPoint;
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::CryptoResolver;
use snow::types::{Cipher, Dh, Hash, Random};
use snow::Error;

const TAGLEN: usize = 16;

/// Resolves only ChaChaPoly + BLAKE2s + X25519; everything else is `None`.
pub struct ShimResolver;

impl CryptoResolver for ShimResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(ShimRng::default()))
    }
    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        match *choice {
            DHChoice::Curve25519 => Some(Box::new(Dh25519::default())),
            _ => None,
        }
    }
    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        match *choice {
            HashChoice::Blake2s => Some(Box::new(HashBLAKE2s::default())),
            _ => None,
        }
    }
    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        match *choice {
            CipherChoice::ChaChaPoly => Some(Box::new(CipherChaChaPoly::default())),
            _ => None,
        }
    }
}

// ── RNG ──────────────────────────────────────────────────────────────────
#[derive(Default)]
struct ShimRng(rand_core::OsRng);
impl rand_core::RngCore for ShimRng {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.fill_bytes(dest)
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.0.try_fill_bytes(dest)
    }
}
impl rand_core::CryptoRng for ShimRng {}
impl Random for ShimRng {}

// ── X25519 ───────────────────────────────────────────────────────────────
#[derive(Default)]
struct Dh25519 {
    privkey: [u8; 32],
    pubkey: [u8; 32],
}
impl Dh25519 {
    fn derive_pubkey(&mut self) {
        self.pubkey = MontgomeryPoint::mul_base_clamped(self.privkey).to_bytes();
    }
}
impl Dh for Dh25519 {
    fn name(&self) -> &'static str {
        "25519"
    }
    fn pub_len(&self) -> usize {
        32
    }
    fn priv_len(&self) -> usize {
        32
    }
    fn set(&mut self, privkey: &[u8]) {
        self.privkey[..privkey.len()].copy_from_slice(privkey);
        self.derive_pubkey();
    }
    fn generate(&mut self, rng: &mut dyn Random) {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        self.privkey = bytes;
        self.derive_pubkey();
    }
    fn pubkey(&self) -> &[u8] {
        &self.pubkey
    }
    fn privkey(&self) -> &[u8] {
        &self.privkey
    }
    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), Error> {
        let mut p = [0u8; 32];
        p.copy_from_slice(&pubkey[..32]);
        let result = MontgomeryPoint(p).mul_clamped(self.privkey).to_bytes();
        out[..result.len()].copy_from_slice(&result);
        Ok(())
    }
}

// ── ChaCha20-Poly1305 ─────────────────────────────────────────────────────
#[derive(Default)]
struct CipherChaChaPoly {
    key: [u8; 32],
}
impl Cipher for CipherChaChaPoly {
    fn name(&self) -> &'static str {
        "ChaChaPoly"
    }
    fn set(&mut self, key: &[u8]) {
        self.key[..key.len()].copy_from_slice(key);
    }
    fn encrypt(&self, nonce: u64, authtext: &[u8], plaintext: &[u8], out: &mut [u8]) -> usize {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&nonce.to_le_bytes());
        out[..plaintext.len()].copy_from_slice(plaintext);
        let tag = ChaCha20Poly1305::new(&self.key.into())
            .encrypt_in_place_detached(&nonce_bytes.into(), authtext, &mut out[0..plaintext.len()])
            .unwrap();
        out[plaintext.len()..][..tag.len()].copy_from_slice(&tag);
        plaintext.len() + tag.len()
    }
    fn decrypt(&self, nonce: u64, authtext: &[u8], ciphertext: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[4..].copy_from_slice(&nonce.to_le_bytes());
        let message_len = ciphertext.len() - TAGLEN;
        out[..message_len].copy_from_slice(&ciphertext[..message_len]);
        ChaCha20Poly1305::new(&self.key.into())
            .decrypt_in_place_detached(
                &nonce_bytes.into(),
                authtext,
                &mut out[..message_len],
                ciphertext[message_len..].into(),
            )
            .map_err(|_| Error::Decrypt)?;
        Ok(message_len)
    }
}

// ── BLAKE2s ────────────────────────────────────────────────────────────────
#[derive(Default)]
struct HashBLAKE2s {
    hasher: Blake2s256,
}
impl Hash for HashBLAKE2s {
    fn name(&self) -> &'static str {
        "BLAKE2s"
    }
    fn block_len(&self) -> usize {
        64
    }
    fn hash_len(&self) -> usize {
        32
    }
    fn reset(&mut self) {
        self.hasher = Blake2s256::default();
    }
    fn input(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }
    fn result(&mut self, out: &mut [u8]) {
        let hash = self.hasher.finalize_reset();
        out[..32].copy_from_slice(&hash);
    }
}
