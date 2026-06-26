// trust/store.rs - Persistent Trust Store using Sled

use sled::{Db, Tree};
use std::path::Path;
use thiserror::Error;

use super::identity::{AgentIdentity, ExportedIdentity, Fingerprint};
use super::signature::{SignedIdentity, TrustSignature};
use super::web::{TrustConfig, WebOfTrust};

/// Errors related to trust store operations
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sled::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Identity not found: {0}")]
    IdentityNotFound(String),

    #[error("Store not initialized")]
    NotInitialized,
}

/// Tree names for different data types
const TREE_IDENTITIES: &str = "identities";
const TREE_SIGNATURES: &str = "signatures";
const TREE_OWNER_TRUST: &str = "owner_trust";
const TREE_CONFIG: &str = "config";
const TREE_SELF: &str = "self_identity";

/// Persistent trust store backed by sled
pub struct TrustStore {
    db: Db,
    identities: Tree,
    signatures: Tree,
    owner_trust: Tree,
    config: Tree,
    self_identity: Tree,
}

impl TrustStore {
    /// Open or create a trust store at the given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StoreError> {
        let db = sled::open(path)?;

        let identities = db.open_tree(TREE_IDENTITIES)?;
        let signatures = db.open_tree(TREE_SIGNATURES)?;
        let owner_trust = db.open_tree(TREE_OWNER_TRUST)?;
        let config = db.open_tree(TREE_CONFIG)?;
        let self_identity = db.open_tree(TREE_SELF)?;

        Ok(Self {
            db,
            identities,
            signatures,
            owner_trust,
            config,
            self_identity,
        })
    }

    /// Create an in-memory trust store (for testing)
    pub fn in_memory() -> Result<Self, StoreError> {
        let db = sled::Config::new().temporary(true).open()?;

        let identities = db.open_tree(TREE_IDENTITIES)?;
        let signatures = db.open_tree(TREE_SIGNATURES)?;
        let owner_trust = db.open_tree(TREE_OWNER_TRUST)?;
        let config = db.open_tree(TREE_CONFIG)?;
        let self_identity = db.open_tree(TREE_SELF)?;

        Ok(Self {
            db,
            identities,
            signatures,
            owner_trust,
            config,
            self_identity,
        })
    }

    /// Store our own identity (encrypted secret key)
    pub fn store_self_identity(&self, identity: &AgentIdentity) -> Result<(), StoreError> {
        let exported = identity.export();
        let bytes = bincode::serde::encode_to_vec(&exported, bincode::config::standard())
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        self.self_identity.insert(b"self", bytes)?;
        self.db.flush()?;
        Ok(())
    }

    /// Load our own identity
    pub fn load_self_identity(&self) -> Result<Option<AgentIdentity>, StoreError> {
        match self.self_identity.get(b"self")? {
            Some(bytes) => {
                let (exported, _): (ExportedIdentity, _) =
                    bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                        .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                let identity = AgentIdentity::import(&exported)
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                Ok(Some(identity))
            }
            None => Ok(None),
        }
    }

    /// Store a signed identity
    pub fn store_identity(&self, identity: &SignedIdentity) -> Result<(), StoreError> {
        let fp = identity.fingerprint();
        let bytes = bincode::serde::encode_to_vec(identity, bincode::config::standard())
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        self.identities.insert(fp.as_bytes(), bytes)?;
        Ok(())
    }

    /// Load a signed identity by fingerprint
    pub fn load_identity(&self, fingerprint: &Fingerprint) -> Result<Option<SignedIdentity>, StoreError> {
        match self.identities.get(fingerprint.as_bytes())? {
            Some(bytes) => {
                let (identity, _): (SignedIdentity, _) =
                    bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                        .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                Ok(Some(identity))
            }
            None => Ok(None),
        }
    }

    /// Load all identities
    pub fn load_all_identities(&self) -> Result<Vec<SignedIdentity>, StoreError> {
        let mut identities = Vec::new();
        for result in self.identities.iter() {
            let (_, bytes) = result?;
            let (identity, _): (SignedIdentity, _) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?;
            identities.push(identity);
        }
        Ok(identities)
    }

    /// Delete an identity
    pub fn delete_identity(&self, fingerprint: &Fingerprint) -> Result<(), StoreError> {
        self.identities.remove(fingerprint.as_bytes())?;
        Ok(())
    }

    /// Store a trust signature
    pub fn store_signature(&self, signature: &TrustSignature) -> Result<(), StoreError> {
        // Key is signer:subject for efficient lookups
        let key = format!("{}:{}", signature.signer, signature.subject);
        let bytes = bincode::serde::encode_to_vec(signature, bincode::config::standard())
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        self.signatures.insert(key.as_bytes(), bytes)?;
        Ok(())
    }

    /// Load signatures for a subject
    pub fn load_signatures_for(&self, subject: &Fingerprint) -> Result<Vec<TrustSignature>, StoreError> {
        let mut signatures = Vec::new();
        let suffix = format!(":{}", subject);

        for result in self.signatures.iter() {
            let (key, bytes) = result?;
            let key_str = String::from_utf8_lossy(&key);
            if key_str.ends_with(&suffix) {
                let (signature, _): (TrustSignature, _) =
                    bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                        .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                signatures.push(signature);
            }
        }
        Ok(signatures)
    }

    /// Store owner trust level
    pub fn store_owner_trust(
        &self,
        fingerprint: &Fingerprint,
        level: super::signature::TrustLevel,
    ) -> Result<(), StoreError> {
        let bytes = bincode::serde::encode_to_vec(&level, bincode::config::standard())
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        self.owner_trust.insert(fingerprint.as_bytes(), bytes)?;
        Ok(())
    }

    /// Load owner trust level
    pub fn load_owner_trust(
        &self,
        fingerprint: &Fingerprint,
    ) -> Result<Option<super::signature::TrustLevel>, StoreError> {
        match self.owner_trust.get(fingerprint.as_bytes())? {
            Some(bytes) => {
                let (level, _): (super::signature::TrustLevel, _) =
                    bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                        .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                Ok(Some(level))
            }
            None => Ok(None),
        }
    }

    /// Load all owner trust settings
    pub fn load_all_owner_trust(
        &self,
    ) -> Result<std::collections::HashMap<Fingerprint, super::signature::TrustLevel>, StoreError> {
        let mut trust_map = std::collections::HashMap::new();
        for result in self.owner_trust.iter() {
            let (key, bytes) = result?;
            let fingerprint = Fingerprint::from_bytes(key.as_ref().try_into().unwrap());
            let (level, _): (super::signature::TrustLevel, _) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?;
            trust_map.insert(fingerprint, level);
        }
        Ok(trust_map)
    }

    /// Store trust configuration
    pub fn store_config(&self, config: &TrustConfig) -> Result<(), StoreError> {
        let bytes = bincode::serde::encode_to_vec(config, bincode::config::standard())
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        self.config.insert(b"config", bytes)?;
        Ok(())
    }

    /// Load trust configuration
    pub fn load_config(&self) -> Result<Option<TrustConfig>, StoreError> {
        match self.config.get(b"config")? {
            Some(bytes) => {
                let (config, _): (TrustConfig, _) =
                    bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                        .map_err(|e| StoreError::SerializationError(e.to_string()))?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Load entire web of trust from store
    pub fn load_web_of_trust(&self) -> Result<WebOfTrust, StoreError> {
        let config = self.load_config()?.unwrap_or_default();
        let mut wot = WebOfTrust::new(config);

        // Load self identity fingerprint
        if let Some(self_identity) = self.load_self_identity()? {
            wot.set_self(self_identity.fingerprint().clone());
        }

        // Load all identities
        for identity in self.load_all_identities()? {
            wot.add_identity(identity);
        }

        // Load owner trust
        for (fp, level) in self.load_all_owner_trust()? {
            wot.set_owner_trust(fp, level);
        }

        Ok(wot)
    }

    /// Save entire web of trust to store
    pub fn save_web_of_trust(&self, wot: &WebOfTrust) -> Result<(), StoreError> {
        let export = wot.export();

        // Store config
        self.store_config(&export.config)?;

        // Store identities
        for identity in &export.identities {
            self.store_identity(identity)?;
        }

        // Store owner trust
        for (fp, level) in &export.owner_trust {
            self.store_owner_trust(fp, *level)?;
        }

        self.db.flush()?;
        Ok(())
    }

    /// Flush all pending writes
    pub fn flush(&self) -> Result<(), StoreError> {
        self.db.flush()?;
        Ok(())
    }

    /// Get database size in bytes
    pub fn size_on_disk(&self) -> u64 {
        self.db.size_on_disk().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::signature::{CertificationType, TrustLevel};

    #[test]
    fn test_store_self_identity() {
        let store = TrustStore::in_memory().unwrap();
        let identity = AgentIdentity::generate("test".into());

        store.store_self_identity(&identity).unwrap();

        let loaded = store.load_self_identity().unwrap().unwrap();
        assert_eq!(identity.fingerprint(), loaded.fingerprint());
    }

    #[test]
    fn test_store_signed_identity() {
        let store = TrustStore::in_memory().unwrap();
        let identity = AgentIdentity::generate("test".into());
        let signed = SignedIdentity::new(&identity, vec!["cap1".into()]);

        store.store_identity(&signed).unwrap();

        let loaded = store.load_identity(signed.fingerprint()).unwrap().unwrap();
        assert_eq!(signed.fingerprint(), loaded.fingerprint());
    }

    #[test]
    fn test_store_owner_trust() {
        let store = TrustStore::in_memory().unwrap();
        let identity = AgentIdentity::generate("test".into());

        store.store_owner_trust(identity.fingerprint(), TrustLevel::Full).unwrap();

        let loaded = store.load_owner_trust(identity.fingerprint()).unwrap().unwrap();
        assert_eq!(loaded, TrustLevel::Full);
    }

    #[test]
    fn test_load_web_of_trust() {
        let store = TrustStore::in_memory().unwrap();

        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());

        // Store Alice's identity
        store.store_self_identity(&alice).unwrap();
        store.store_identity(&SignedIdentity::new(&alice, vec![])).unwrap();
        store.store_owner_trust(alice.fingerprint(), TrustLevel::Ultimate).unwrap();

        // Store Bob's identity
        let mut bob_signed = SignedIdentity::new(&bob, vec![]);
        bob_signed.add_endorsement(TrustSignature::create(
            &alice,
            &bob.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        ));
        store.store_identity(&bob_signed).unwrap();

        // Load and verify
        let wot = store.load_web_of_trust().unwrap();
        assert!(wot.get_identity(alice.fingerprint()).is_some());
        assert!(wot.get_identity(bob.fingerprint()).is_some());
    }
}
