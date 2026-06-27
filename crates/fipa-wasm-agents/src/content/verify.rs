//! Content-language verification seam (FIPA layer — content-agnostic).
//!
//! The FIPA runtime decodes the ACL envelope, then asks a [`ContentVerifier`]
//! to vet the *content* before the agent acts on it. The runtime knows nothing
//! about UNL (or any specific content language) — a verifier handles exactly the
//! languages it understands and returns `Ok` for anything else. UNL plugs in as
//! one implementation (`content::unl::UnlVerifier`); the dependency points
//! strictly UNL → FIPA.

use crate::proto::{AclMessage, AgentId, Performative};
use std::collections::HashMap;

/// Vets a message's content before delivery. Content-language agnostic.
pub trait ContentVerifier: Send + Sync {
    /// `Ok(())` to deliver the message to the agent; `Err(reason)` to reject it
    /// with a `not-understood` reply (the agent has no way to act on it).
    fn verify(&self, msg: &AclMessage) -> Result<(), String>;
}

/// Build the `not-understood` reply for a message that could not be vetted —
/// threaded back to the original sender, echoing the conversation. Purely a
/// FIPA-envelope operation; carries no content and no content language.
pub fn not_understood(msg: &AclMessage, from: &str) -> AclMessage {
    AclMessage {
        message_id: format!("nu-{}", uuid::Uuid::new_v4()),
        performative: Performative::NotUnderstood as i32,
        sender: Some(AgentId {
            name: from.to_string(),
            addresses: Vec::new(),
            resolvers: Vec::new(),
        }),
        receivers: msg.sender.clone().into_iter().collect(),
        reply_to: None,
        protocol: msg.protocol,
        conversation_id: msg.conversation_id.clone(),
        in_reply_to: msg.reply_with.clone(),
        reply_with: None,
        reply_by: None,
        language: None,
        encoding: None,
        ontology: None,
        content: Vec::new(),
        user_properties: HashMap::new(),
    }
}
