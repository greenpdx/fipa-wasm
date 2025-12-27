// trust/identity.rs - Agent Identity with Ed25519 Keys

use ed25519_dalek::{
    Signature, Signer, SigningKey, Verifier, VerifyingKey,
    SECRET_KEY_LENGTH, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::fmt;
use thiserror::Error;

/// Errors related to identity operations
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Invalid key length: expected {expected}, got {got}")]
    InvalidKeyLength { expected: usize, got: usize },

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Key generation failed: {0}")]
    KeyGenerationFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// A unique fingerprint derived from a public key
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Create fingerprint from public key bytes
    pub fn from_public_key(public_key: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(public_key);
        Self(hasher.finalize().into())
    }

    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Short form for display (first 8 bytes as hex)
    pub fn short(&self) -> String {
        hex::encode(&self.0[..8])
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

/// Agent identity containing cryptographic keys
pub struct AgentIdentity {
    /// The signing (private) key
    signing_key: SigningKey,

    /// The verifying (public) key
    verifying_key: VerifyingKey,

    /// Cached fingerprint
    fingerprint: Fingerprint,

    /// Agent name (human-readable identifier)
    name: String,

    /// Creation timestamp
    created_at: i64,
}

impl AgentIdentity {
    /// Generate a new random identity
    pub fn generate(name: String) -> Self {
        let mut secret_bytes = [0u8; SECRET_KEY_LENGTH];
        rand::rng().fill_bytes(&mut secret_bytes);

        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        let fingerprint = Fingerprint::from_public_key(verifying_key.as_bytes());

        Self {
            signing_key,
            verifying_key,
            fingerprint,
            name,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    /// Create from existing secret key bytes
    pub fn from_secret_key(
        secret_bytes: &[u8],
        name: String,
    ) -> Result<Self, IdentityError> {
        if secret_bytes.len() != SECRET_KEY_LENGTH {
            return Err(IdentityError::InvalidKeyLength {
                expected: SECRET_KEY_LENGTH,
                got: secret_bytes.len(),
            });
        }

        let mut key_bytes = [0u8; SECRET_KEY_LENGTH];
        key_bytes.copy_from_slice(secret_bytes);

        let signing_key = SigningKey::from_bytes(&key_bytes);
        let verifying_key = signing_key.verifying_key();
        let fingerprint = Fingerprint::from_public_key(verifying_key.as_bytes());

        Ok(Self {
            signing_key,
            verifying_key,
            fingerprint,
            name,
            created_at: chrono::Utc::now().timestamp(),
        })
    }

    /// Get the agent name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the fingerprint
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// Get the public key bytes
    pub fn public_key_bytes(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.verifying_key.to_bytes()
    }

    /// Get the secret key bytes (be careful with this!)
    pub fn secret_key_bytes(&self) -> [u8; SECRET_KEY_LENGTH] {
        self.signing_key.to_bytes()
    }

    /// Sign arbitrary data
    pub fn sign(&self, data: &[u8]) -> [u8; SIGNATURE_LENGTH] {
        self.signing_key.sign(data).to_bytes()
    }

    /// Create a public identity (for sharing)
    pub fn to_public(&self) -> PublicIdentity {
        PublicIdentity {
            public_key: self.verifying_key.to_bytes(),
            fingerprint: self.fingerprint.clone(),
            name: self.name.clone(),
            created_at: self.created_at,
        }
    }

    /// Export to serializable format
    pub fn export(&self) -> ExportedIdentity {
        ExportedIdentity {
            secret_key: self.signing_key.to_bytes().to_vec(),
            name: self.name.clone(),
            created_at: self.created_at,
        }
    }

    /// Import from serializable format
    pub fn import(exported: &ExportedIdentity) -> Result<Self, IdentityError> {
        let mut identity = Self::from_secret_key(&exported.secret_key, exported.name.clone())?;
        identity.created_at = exported.created_at;
        Ok(identity)
    }
}

impl fmt::Debug for AgentIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentIdentity")
            .field("name", &self.name)
            .field("fingerprint", &self.fingerprint.short())
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Public identity that can be shared
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicIdentity {
    /// Public key bytes
    pub public_key: [u8; PUBLIC_KEY_LENGTH],

    /// Fingerprint
    pub fingerprint: Fingerprint,

    /// Agent name
    pub name: String,

    /// Creation timestamp
    pub created_at: i64,
}

impl PublicIdentity {
    /// Verify a signature
    pub fn verify(&self, data: &[u8], signature: &[u8]) -> Result<(), IdentityError> {
        if signature.len() != SIGNATURE_LENGTH {
            return Err(IdentityError::InvalidKeyLength {
                expected: SIGNATURE_LENGTH,
                got: signature.len(),
            });
        }

        let verifying_key = VerifyingKey::from_bytes(&self.public_key)
            .map_err(|_| IdentityError::InvalidSignature)?;

        let mut sig_bytes = [0u8; SIGNATURE_LENGTH];
        sig_bytes.copy_from_slice(signature);
        let sig = Signature::from_bytes(&sig_bytes);

        verifying_key
            .verify(data, &sig)
            .map_err(|_| IdentityError::InvalidSignature)
    }

    /// Create from public key bytes
    pub fn from_public_key(public_key: [u8; PUBLIC_KEY_LENGTH], name: String) -> Self {
        let fingerprint = Fingerprint::from_public_key(&public_key);
        Self {
            public_key,
            fingerprint,
            name,
            created_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// Exportable identity format (includes secret key)
#[derive(Serialize, Deserialize)]
pub struct ExportedIdentity {
    pub secret_key: Vec<u8>,
    pub name: String,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_identity() {
        let identity = AgentIdentity::generate("test-agent".into());
        assert_eq!(identity.name(), "test-agent");
        assert_eq!(identity.fingerprint().as_bytes().len(), 32);
    }

    #[test]
    fn test_sign_and_verify() {
        let identity = AgentIdentity::generate("signer".into());
        let data = b"hello world";

        let signature = identity.sign(data);
        let public = identity.to_public();

        assert!(public.verify(data, &signature).is_ok());
        assert!(public.verify(b"wrong data", &signature).is_err());
    }

    #[test]
    fn test_export_import() {
        let identity = AgentIdentity::generate("exportable".into());
        let exported = identity.export();
        let imported = AgentIdentity::import(&exported).unwrap();

        assert_eq!(identity.name(), imported.name());
        assert_eq!(identity.fingerprint(), imported.fingerprint());
    }

    #[test]
    fn test_fingerprint_display() {
        let identity = AgentIdentity::generate("test".into());
        let fp = identity.fingerprint();

        assert_eq!(fp.short().len(), 16); // 8 bytes = 16 hex chars
        assert_eq!(fp.to_string().len(), 64); // 32 bytes = 64 hex chars
    }
}
