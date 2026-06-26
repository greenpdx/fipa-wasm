// trust/web.rs - Web of Trust Graph and Trust Calculation

use std::collections::{HashMap, VecDeque};
use serde::{Deserialize, Serialize};

use super::identity::Fingerprint;
use super::signature::{SignedIdentity, TrustLevel};

/// Configuration for trust calculation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustConfig {
    /// Maximum path length for trust propagation
    pub max_path_length: usize,

    /// Number of marginal signatures needed to equal one full signature
    pub marginals_needed: usize,

    /// Number of full signatures needed for full validity
    pub completes_needed: usize,

    /// Whether to allow trust through marginal introducers
    pub allow_marginal_introducers: bool,

    /// Decay factor per hop (1.0 = no decay)
    pub trust_decay: f64,
}

impl Default for TrustConfig {
    fn default() -> Self {
        Self {
            max_path_length: 5,
            marginals_needed: 3,
            completes_needed: 1,
            allow_marginal_introducers: true,
            trust_decay: 0.8,
        }
    }
}

/// Calculated trust validity for an identity
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustValidity {
    /// Unknown identity
    Unknown,

    /// Explicitly revoked/untrusted
    Revoked,

    /// Not enough signatures
    Undefined,

    /// Marginally valid (some trust, not fully verified)
    Marginal,

    /// Fully valid (meets trust requirements)
    Full,

    /// Ultimately trusted (own keys or explicitly set)
    Ultimate,
}

/// A trust path from source to target
#[derive(Clone, Debug)]
pub struct TrustPath {
    /// Fingerprints in the path (source -> ... -> target)
    pub path: Vec<Fingerprint>,

    /// Trust level at each hop
    pub levels: Vec<TrustLevel>,

    /// Calculated effective trust
    pub effective_trust: f64,
}

impl TrustPath {
    /// Calculate effective trust with decay
    pub fn calculate_effective_trust(&self, decay: f64) -> f64 {
        let mut trust = 1.0;
        for (i, level) in self.levels.iter().enumerate() {
            let level_weight = match level {
                TrustLevel::Unknown => 0.0,
                TrustLevel::Untrusted => 0.0,
                TrustLevel::Marginal => 0.5,
                TrustLevel::Full => 1.0,
                TrustLevel::Ultimate => 1.0,
            };
            trust *= level_weight * decay.powi(i as i32);
        }
        trust
    }
}

/// The Web of Trust graph
pub struct WebOfTrust {
    /// Known identities indexed by fingerprint
    identities: HashMap<Fingerprint, SignedIdentity>,

    /// Trust assignments from our perspective (fingerprint -> trust level)
    owner_trust: HashMap<Fingerprint, TrustLevel>,

    /// Our own identity fingerprint
    self_fingerprint: Option<Fingerprint>,

    /// Trust calculation configuration
    config: TrustConfig,

    /// Cached validity calculations
    validity_cache: HashMap<Fingerprint, TrustValidity>,
}

impl WebOfTrust {
    /// Create a new empty web of trust
    pub fn new(config: TrustConfig) -> Self {
        Self {
            identities: HashMap::new(),
            owner_trust: HashMap::new(),
            self_fingerprint: None,
            config,
            validity_cache: HashMap::new(),
        }
    }

    /// Set our own identity
    pub fn set_self(&mut self, fingerprint: Fingerprint) {
        self.self_fingerprint = Some(fingerprint.clone());
        self.owner_trust.insert(fingerprint, TrustLevel::Ultimate);
        self.invalidate_cache();
    }

    /// Add or update an identity
    pub fn add_identity(&mut self, identity: SignedIdentity) {
        let fp = identity.fingerprint().clone();
        self.identities.insert(fp, identity);
        self.invalidate_cache();
    }

    /// Get an identity by fingerprint
    pub fn get_identity(&self, fingerprint: &Fingerprint) -> Option<&SignedIdentity> {
        self.identities.get(fingerprint)
    }

    /// Set owner trust level for a fingerprint
    pub fn set_owner_trust(&mut self, fingerprint: Fingerprint, level: TrustLevel) {
        self.owner_trust.insert(fingerprint, level);
        self.invalidate_cache();
    }

    /// Get owner trust level
    pub fn get_owner_trust(&self, fingerprint: &Fingerprint) -> TrustLevel {
        self.owner_trust.get(fingerprint).copied().unwrap_or(TrustLevel::Unknown)
    }

    /// Invalidate the validity cache
    fn invalidate_cache(&mut self) {
        self.validity_cache.clear();
    }

    /// Calculate validity for an identity
    pub fn calculate_validity(&mut self, fingerprint: &Fingerprint) -> TrustValidity {
        // Check cache
        if let Some(validity) = self.validity_cache.get(fingerprint) {
            return validity.clone();
        }

        let validity = self.calculate_validity_uncached(fingerprint);
        self.validity_cache.insert(fingerprint.clone(), validity.clone());
        validity
    }

    fn calculate_validity_uncached(&self, fingerprint: &Fingerprint) -> TrustValidity {
        // Check if it's our own key
        if self.self_fingerprint.as_ref() == Some(fingerprint) {
            return TrustValidity::Ultimate;
        }

        // Check owner trust
        match self.owner_trust.get(fingerprint) {
            Some(TrustLevel::Ultimate) => return TrustValidity::Ultimate,
            Some(TrustLevel::Untrusted) => return TrustValidity::Revoked,
            _ => {}
        }

        // Get the identity
        let identity = match self.identities.get(fingerprint) {
            Some(id) => id,
            None => return TrustValidity::Unknown,
        };

        // Count valid signatures from trusted introducers
        let mut full_count = 0;
        let mut marginal_count = 0;

        for endorsement in identity.valid_endorsements() {
            // Check if the signer is trusted as an introducer
            let signer_trust = self.get_owner_trust(&endorsement.signer);

            if !signer_trust.can_introduce() {
                // Try to find trust paths to this signer
                let paths = self.find_trust_paths(&endorsement.signer);
                if paths.is_empty() {
                    continue;
                }

                // Use the best path
                let best_trust = paths.iter()
                    .map(|p| p.effective_trust)
                    .max_by(|a, b| a.partial_cmp(b).unwrap())
                    .unwrap_or(0.0);

                if best_trust >= 0.8 {
                    match endorsement.trust_level {
                        TrustLevel::Full | TrustLevel::Ultimate => full_count += 1,
                        TrustLevel::Marginal => marginal_count += 1,
                        _ => {}
                    }
                } else if best_trust >= 0.4 && self.config.allow_marginal_introducers {
                    marginal_count += 1;
                }
            } else {
                // Direct trust from owner
                match endorsement.trust_level {
                    TrustLevel::Full | TrustLevel::Ultimate => full_count += 1,
                    TrustLevel::Marginal => marginal_count += 1,
                    _ => {}
                }
            }
        }

        // Apply PGP-like trust model
        if full_count >= self.config.completes_needed {
            TrustValidity::Full
        } else if marginal_count >= self.config.marginals_needed {
            TrustValidity::Marginal
        } else if full_count > 0 || marginal_count > 0 {
            TrustValidity::Marginal
        } else {
            TrustValidity::Undefined
        }
    }

    /// Find all trust paths from self to target
    pub fn find_trust_paths(&self, target: &Fingerprint) -> Vec<TrustPath> {
        let self_fp = match &self.self_fingerprint {
            Some(fp) => fp,
            None => return vec![],
        };

        if self_fp == target {
            return vec![TrustPath {
                path: vec![self_fp.clone()],
                levels: vec![],
                effective_trust: 1.0,
            }];
        }

        // BFS to find paths
        let mut paths = Vec::new();
        let mut queue: VecDeque<(Vec<Fingerprint>, Vec<TrustLevel>)> = VecDeque::new();

        queue.push_back((vec![self_fp.clone()], vec![]));

        while let Some((current_path, current_levels)) = queue.pop_front() {
            if current_path.len() > self.config.max_path_length {
                continue;
            }

            let current = current_path.last().unwrap();

            // Get all identities signed by current
            for (fp, identity) in &self.identities {
                if current_path.contains(fp) {
                    continue; // Avoid cycles
                }

                // Check if current has endorsed this identity
                for endorsement in identity.valid_endorsements() {
                    if &endorsement.signer == current {
                        let mut new_path = current_path.clone();
                        new_path.push(fp.clone());

                        let mut new_levels = current_levels.clone();
                        new_levels.push(endorsement.trust_level);

                        if fp == target {
                            let mut trust_path = TrustPath {
                                path: new_path,
                                levels: new_levels,
                                effective_trust: 0.0,
                            };
                            trust_path.effective_trust =
                                trust_path.calculate_effective_trust(self.config.trust_decay);
                            paths.push(trust_path);
                        } else {
                            queue.push_back((new_path, new_levels));
                        }
                    }
                }
            }
        }

        // Sort by effective trust (highest first)
        paths.sort_by(|a, b| b.effective_trust.partial_cmp(&a.effective_trust).unwrap());
        paths
    }

    /// Get all identities with a minimum validity level
    pub fn get_valid_identities(&mut self, min_validity: TrustValidity) -> Vec<Fingerprint> {
        let fingerprints: Vec<_> = self.identities.keys().cloned().collect();

        // First calculate validities
        let mut valid_fps = Vec::new();
        for fp in fingerprints {
            let validity = self.calculate_validity(&fp);
            let passes = match (&validity, &min_validity) {
                (TrustValidity::Ultimate, _) => true,
                (TrustValidity::Full, TrustValidity::Full) => true,
                (TrustValidity::Full, TrustValidity::Marginal) => true,
                (TrustValidity::Marginal, TrustValidity::Marginal) => true,
                _ => false,
            };
            if passes {
                valid_fps.push(fp);
            }
        }
        valid_fps
    }

    /// Export the web of trust state
    pub fn export(&self) -> WebOfTrustExport {
        WebOfTrustExport {
            identities: self.identities.values().cloned().collect(),
            owner_trust: self.owner_trust.clone(),
            self_fingerprint: self.self_fingerprint.clone(),
            config: self.config.clone(),
        }
    }

    /// Import web of trust state
    pub fn import(export: WebOfTrustExport) -> Self {
        let mut wot = Self::new(export.config);
        wot.self_fingerprint = export.self_fingerprint;
        wot.owner_trust = export.owner_trust;
        for identity in export.identities {
            wot.identities.insert(identity.fingerprint().clone(), identity);
        }
        wot
    }

    /// Get statistics about the web of trust
    pub fn stats(&self) -> WebOfTrustStats {
        WebOfTrustStats {
            total_identities: self.identities.len(),
            trusted_identities: self.owner_trust
                .values()
                .filter(|l| l.can_introduce())
                .count(),
            total_signatures: self.identities
                .values()
                .map(|i| i.endorsements.len())
                .sum(),
        }
    }
}

/// Exportable web of trust state
#[derive(Serialize, Deserialize)]
pub struct WebOfTrustExport {
    pub identities: Vec<SignedIdentity>,
    pub owner_trust: HashMap<Fingerprint, TrustLevel>,
    pub self_fingerprint: Option<Fingerprint>,
    pub config: TrustConfig,
}

/// Web of trust statistics
#[derive(Debug)]
pub struct WebOfTrustStats {
    pub total_identities: usize,
    pub trusted_identities: usize,
    pub total_signatures: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::identity::AgentIdentity;
    use crate::trust::signature::{CertificationType, TrustSignature};

    #[test]
    fn test_direct_trust() {
        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());

        let mut wot = WebOfTrust::new(TrustConfig::default());
        wot.set_self(alice.fingerprint().clone());
        wot.add_identity(SignedIdentity::new(&alice, vec![]));

        // Create Bob's identity with Alice's endorsement
        let mut bob_signed = SignedIdentity::new(&bob, vec![]);
        bob_signed.add_endorsement(TrustSignature::create(
            &alice,
            &bob.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        ));
        wot.add_identity(bob_signed);

        // Set Alice as trusted introducer
        wot.set_owner_trust(alice.fingerprint().clone(), TrustLevel::Ultimate);

        let validity = wot.calculate_validity(bob.fingerprint());
        assert_eq!(validity, TrustValidity::Full);
    }

    #[test]
    fn test_trust_path() {
        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());
        let charlie = AgentIdentity::generate("charlie".into());

        let mut wot = WebOfTrust::new(TrustConfig::default());
        wot.set_self(alice.fingerprint().clone());
        wot.add_identity(SignedIdentity::new(&alice, vec![]));
        wot.set_owner_trust(alice.fingerprint().clone(), TrustLevel::Ultimate);

        // Alice endorses Bob
        let mut bob_signed = SignedIdentity::new(&bob, vec![]);
        bob_signed.add_endorsement(TrustSignature::create(
            &alice,
            &bob.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        ));
        wot.add_identity(bob_signed);
        wot.set_owner_trust(bob.fingerprint().clone(), TrustLevel::Full);

        // Bob endorses Charlie
        let mut charlie_signed = SignedIdentity::new(&charlie, vec![]);
        charlie_signed.add_endorsement(TrustSignature::create(
            &bob,
            &charlie.to_public(),
            TrustLevel::Full,
            CertificationType::Identity,
            None,
            0,
        ));
        wot.add_identity(charlie_signed);

        // Find paths to Charlie
        let paths = wot.find_trust_paths(charlie.fingerprint());
        assert!(!paths.is_empty());
        assert_eq!(paths[0].path.len(), 3); // Alice -> Bob -> Charlie
    }

    #[test]
    fn test_marginal_trust() {
        let config = TrustConfig {
            marginals_needed: 2,
            completes_needed: 1,
            ..Default::default()
        };

        let alice = AgentIdentity::generate("alice".into());
        let bob = AgentIdentity::generate("bob".into());
        let charlie = AgentIdentity::generate("charlie".into());
        let dave = AgentIdentity::generate("dave".into());

        let mut wot = WebOfTrust::new(config);
        wot.set_self(alice.fingerprint().clone());
        wot.set_owner_trust(alice.fingerprint().clone(), TrustLevel::Ultimate);
        wot.set_owner_trust(bob.fingerprint().clone(), TrustLevel::Full);
        wot.set_owner_trust(charlie.fingerprint().clone(), TrustLevel::Full);

        // Bob and Charlie both give marginal trust to Dave
        let mut dave_signed = SignedIdentity::new(&dave, vec![]);
        dave_signed.add_endorsement(TrustSignature::create(
            &bob,
            &dave.to_public(),
            TrustLevel::Marginal,
            CertificationType::Identity,
            None,
            0,
        ));
        dave_signed.add_endorsement(TrustSignature::create(
            &charlie,
            &dave.to_public(),
            TrustLevel::Marginal,
            CertificationType::Identity,
            None,
            0,
        ));
        wot.add_identity(dave_signed);

        let validity = wot.calculate_validity(dave.fingerprint());
        assert_eq!(validity, TrustValidity::Marginal);
    }
}
