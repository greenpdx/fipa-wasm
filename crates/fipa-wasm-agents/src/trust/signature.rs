// trust/signature.rs - Trust Signatures and Endorsements

use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

use super::identity::{AgentIdentity, Fingerprint, IdentityError, PublicIdentity};

/// Trust level assigned in an endorsement
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Unknown - no trust relationship
    Unknown = 0,

    /// Untrusted - explicitly marked as untrustworthy
    Untrusted = 1,

    /// Marginal - limited trust (e.g., new acquaintance)
    Marginal = 2,

    /// Full - fully trusted (e.g., verified identity)
    Full = 3,

    /// Ultimate - implicitly trusted (e.g., own keys)
    Ultimate = 4,
}

impl TrustLevel {
    /// Convert from integer
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(TrustLevel::Unknown),
            1 => Some(TrustLevel::Untrusted),
            2 => Some(TrustLevel::Marginal),
            3 => Some(TrustLevel::Full),
            4 => Some(TrustLevel::Ultimate),
            _ => None,
        }
    }

    /// Check if this trust level allows signing on behalf
    pub fn can_introduce(&self) -> bool {
        matches!(self, TrustLevel::Full | TrustLevel::Ultimate)
    }
}

impl Default for TrustLevel {
    fn default() -> Self {
        TrustLevel::Unknown
    }
}

/// Certification type - what is being certified
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificationType {
    /// Generic endorsement of identity
    Identity,

    /// Endorsement as a valid agent
    Agent,

    /// Endorsement for a specific capability
    Capability,

    /// Endorsement for a specific service
    Service,

    /// Revocation of previous endorsement
    Revocation,
}

/// A trust signature/endorsement from one agent to another
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustSignature {
    /// Who is signing (endorser)
    pub signer: Fingerprint,

    /// Who is being endorsed (subject)
    pub subject: Fingerprint,

    /// Trust level assigned
    pub trust_level: TrustLevel,

    /// Type of certification
    pub cert_type: CertificationType,

    /// Optional scope/capability being endorsed
    pub scope: Option<String>,

    /// When this signature was created
    pub created_at: i64,

    /// When this signature expires (0 = never)
    pub expires_at: i64,

    /// The cryptographic signature
    pub signature: Vec<u8>,
}

impl TrustSignature {
    /// Create a new trust signature
    pub fn create(
        signer: &AgentIdentity,
        subject: &PublicIdentity,
        trust_level: TrustLevel,
        cert_type: CertificationType,
        scope: Option<String>,
        expires_at: i64,
    ) -> Self {
        let created_at = chrono::Utc::now().timestamp();

        // Create the data to sign
        let mut sig = TrustSignature {
            signer: signer.fingerprint().clone(),
            subject: subject.fingerprint.clone(),
            trust_level,
            cert_type,
            scope,
            created_at,
            expires_at,
            signature: vec![],
        };

        // Sign it
        let data = sig.signable_data();
        sig.signature = signer.sign(&data).to_vec();

        sig
    }

    /// Get the canonical data that is signed
    fn signable_data(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.signer.as_bytes());
        hasher.update(self.subject.as_bytes());
        hasher.update(&[self.trust_level as u8]);
        hasher.update(&[self.cert_type as u8]);
        if let Some(ref scope) = self.scope {
            hasher.update(scope.as_bytes());
        }
        hasher.update(&self.created_at.to_le_bytes());
        hasher.update(&self.expires_at.to_le_bytes());
        hasher.finalize().to_vec()
    }

    /// Verify this signature against a public identity
    pub fn verify(&self, signer_identity: &PublicIdentity) -> Result<(), IdentityError> {
        // Check that signer matches
        if signer_identity.fingerprint != self.signer {
            return Err(IdentityError::InvalidSignature);
        }

        // Check expiration
        if self.expires_at > 0 {
            let now = chrono::Utc::now().timestamp();
            if now > self.expires_at {
                return Err(IdentityError::InvalidSignature);
            }
        }

        // Verify the cryptographic signature
        let data = self.signable_data();
        signer_identity.verify(&data, &self.signature)
    }

    /// Check if this is a revocation
    pub fn is_revocation(&self) -> bool {
        self.cert_type == CertificationType::Revocation
    }

    /// Check if this signature has expired
    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 {
            return false;
        }
        let now = chrono::Utc::now().timestamp();
        now > self.expires_at
    }
}

/// A self-signature (key certification)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfSignature {
    /// The identity being certified
    pub identity: PublicIdentity,

    /// Agent capabilities/claims
    pub capabilities: Vec<String>,

    /// Additional metadata
    pub metadata: std::collections::HashMap<String, String>,

    /// Creation timestamp
    pub created_at: i64,

    /// The signature
    pub signature: Vec<u8>,
}

impl SelfSignature {
    /// Create a self-signature
    pub fn create(
        identity: &AgentIdentity,
        capabilities: Vec<String>,
        metadata: std::collections::HashMap<String, String>,
    ) -> Self {
        let created_at = chrono::Utc::now().timestamp();

        let mut sig = SelfSignature {
            identity: identity.to_public(),
            capabilities,
            metadata,
            created_at,
            signature: vec![],
        };

        let data = sig.signable_data();
        sig.signature = identity.sign(&data).to_vec();

        sig
    }

    /// Get the canonical data that is signed
    fn signable_data(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(&self.identity.public_key);
        hasher.update(self.identity.name.as_bytes());
        for cap in &self.capabilities {
            hasher.update(cap.as_bytes());
        }
        // Sort keys for deterministic hashing
        let mut keys: Vec<_> = self.metadata.keys().collect();
        keys.sort();
        for key in keys {
            hasher.update(key.as_bytes());
            hasher.update(self.metadata.get(key).unwrap().as_bytes());
        }
        hasher.update(&self.created_at.to_le_bytes());
        hasher.finalize().to_vec()
    }

    /// Verify this self-signature
    pub fn verify(&self) -> Result<(), IdentityError> {
        let data = self.signable_data();
        self.identity.verify(&data, &self.signature)
    }
}

/// A complete signed identity package (like a PGP key)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedIdentity {
    /// The self-signature
    pub self_signature: SelfSignature,

    /// Trust signatures from others
    pub endorsements: Vec<TrustSignature>,
}

impl SignedIdentity {
    /// Create a new signed identity
    pub fn new(identity: &AgentIdentity, capabilities: Vec<String>) -> Self {
        let self_signature = SelfSignature::create(
            identity,
            capabilities,
            std::collections::HashMap::new(),
        );

        Self {
            self_signature,
            endorsements: vec![],
        }
    }

    /// Add an endorsement
    pub fn add_endorsement(&mut self, sig: TrustSignature) {
        self.endorsements.push(sig);
    }

    /// Get the public identity
    pub fn public_identity(&self) -> &PublicIdentity {
        &self.self_signature.identity
    }

    /// Get the fingerprint
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.self_signature.identity.fingerprint
    }

    /// Get valid endorsements (not expired, not revoked)
    pub fn valid_endorsements(&self) -> Vec<&TrustSignature> {
        let revocations: std::collections::HashSet<_> = self.endorsements
            .iter()
            .filter(|s| s.is_revocation())
            .map(|s| &s.signer)
            .collect();

        self.endorsements
            .iter()
            .filter(|s| !s.is_revocation() && !s.is_expired())
            .filter(|s| !revocations.contains(&s.signer))
            .collect()
    }

    /// Count endorsements at each trust level
    pub fn endorsement_counts(&self) -> std::collections::HashMap<TrustLevel, usize> {
        let mut counts = std::collections::HashMap::new();
        for sig in self.valid_endorsements() {
            *counts.entry(sig.trust_level).or_insert(0) += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_signature() {
        let signer = AgentIdentity::generate("signer".into());
        let subject = AgentIdentity::generate("subject".into());

        let sig = TrustSignature::create(
            &signer,
            &subject.to_public(),
            TrustLevel::Full,
            CertificationType::Agent,
            None,
            0,
        );

        assert!(sig.verify(&signer.to_public()).is_ok());
        assert!(!sig.is_expired());
        assert!(!sig.is_revocation());
    }

    #[test]
    fn test_self_signature() {
        let identity = AgentIdentity::generate("self".into());

        let sig = SelfSignature::create(
            &identity,
            vec!["messaging".into(), "compute".into()],
            std::collections::HashMap::new(),
        );

        assert!(sig.verify().is_ok());
    }

    #[test]
    fn test_signed_identity() {
        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());

        let mut signed = SignedIdentity::new(&alice, vec!["agent".into()]);

        let endorsement = TrustSignature::create(
            &bob,
            &alice.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        );

        signed.add_endorsement(endorsement);

        assert_eq!(signed.valid_endorsements().len(), 1);
        let counts = signed.endorsement_counts();
        assert_eq!(counts.get(&TrustLevel::Full), Some(&1));
    }

    #[test]
    fn test_revocation() {
        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());

        let mut signed = SignedIdentity::new(&alice, vec![]);

        // Bob endorses Alice
        let endorsement = TrustSignature::create(
            &bob,
            &alice.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        );
        signed.add_endorsement(endorsement);

        // Bob revokes endorsement
        let revocation = TrustSignature::create(
            &bob,
            &alice.to_public(),
            TrustLevel::Untrusted,
            CertificationType::Revocation,
            None,
            0,
        );
        signed.add_endorsement(revocation);

        // Should have no valid endorsements now
        assert_eq!(signed.valid_endorsements().len(), 0);
    }
}
