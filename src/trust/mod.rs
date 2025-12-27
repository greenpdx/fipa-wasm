// trust/mod.rs - Web of Trust Module

//! Web of Trust implementation for agent identity and trust management.
//!
//! This module provides a PGP-like web of trust for FIPA agents:
//!
//! - **Identity**: Ed25519 key pairs for agent authentication
//! - **Signatures**: Trust endorsements between agents
//! - **Web**: Trust graph with path-based validity calculation
//! - **Store**: Persistent storage using sled
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::trust::*;
//!
//! // Create an identity
//! let alice = AgentIdentity::generate("alice".into());
//!
//! // Create web of trust
//! let mut wot = WebOfTrust::new(TrustConfig::default());
//! wot.set_self(alice.fingerprint().clone());
//!
//! // Create and endorse another agent
//! let bob = AgentIdentity::generate("bob".into());
//! let mut bob_signed = SignedIdentity::new(&bob, vec!["messaging".into()]);
//!
//! bob_signed.add_endorsement(TrustSignature::create(
//!     &alice,
//!     &bob.to_public(),
//!     TrustLevel::Full,
//!     CertificationType::Agent,
//!     None,
//!     0,
//! ));
//!
//! wot.add_identity(bob_signed);
//!
//! // Calculate validity
//! let validity = wot.calculate_validity(bob.fingerprint());
//! ```

mod identity;
mod signature;
mod store;
mod web;

pub use identity::{
    AgentIdentity, ExportedIdentity, Fingerprint, IdentityError, PublicIdentity,
};

pub use signature::{
    CertificationType, SelfSignature, SignedIdentity, TrustLevel, TrustSignature,
};

pub use store::{StoreError, TrustStore};

pub use web::{
    TrustConfig, TrustPath, TrustValidity, WebOfTrust, WebOfTrustExport, WebOfTrustStats,
};
