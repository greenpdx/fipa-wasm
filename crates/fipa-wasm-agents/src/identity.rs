//! Agent identity — FIPA AID, with a UUID name.
//!
//! FIPA's Agent Identifier is `{ name (a globally-unique GUID), addresses,
//! resolvers }`. We make `name` a **UUID** (location-independent, so a *mobile*
//! agent keeps its identity across migrations — `addresses` are resolved via AMS,
//! never baked in). On top of the AID:
//!
//! - **instance UUID** — the AID `name`, unique per running agent;
//! - **type {UUID, desc}** — the *kind* of agent, carried in the bundle `HEAD`
//!   block; many instances share a type;
//! - **friendly name** — a display alias for logs/demos only, never the identity.
//!
//! Lifecycle:
//! - **ephemeral / mobile** agents (e.g. a buyer per purchase) **mint** a fresh
//!   instance UUID at spawn ([`AgentId::spawn`]);
//! - **persistent infrastructure** agents (DF/AMS/PA) **persist** their UUID
//!   ([`AgentId::load_or_mint`]) so long-lived references (a held order naming the
//!   seller) survive a restart.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The *kind* of an agent: a stable UUID + a human description.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentType {
    pub id: Uuid,
    pub desc: String,
}

/// The bundle `HEAD` block: the type header (+ an optional friendly name). The
/// instance UUID is *not* here — it is minted (or loaded) at spawn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    #[serde(rename = "type")]
    pub type_id: Uuid,
    pub desc: String,
    #[serde(default)]
    pub name: Option<String>,
}

impl Header {
    pub fn from_block(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
    pub fn to_block(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// A spawned agent's identity (a FIPA AID with a UUID name).
#[derive(Clone, Debug)]
pub struct AgentId {
    /// The AID `name`: this running instance.
    pub instance: Uuid,
    pub agent_type: AgentType,
    /// Display-only friendly name.
    pub name: Option<String>,
}

impl AgentId {
    fn with_instance(instance: Uuid, header: &Header) -> Self {
        AgentId {
            instance,
            agent_type: AgentType { id: header.type_id, desc: header.desc.clone() },
            name: header.name.clone(),
        }
    }

    /// Mint a fresh instance identity from a type header — ephemeral/mobile agents.
    pub fn spawn(header: &Header) -> Self {
        Self::with_instance(Uuid::new_v4(), header)
    }

    /// Load a persisted instance UUID from `path`, or mint + persist one if
    /// absent — long-lived infrastructure agents (stable identity across restart).
    pub fn load_or_mint(header: &Header, path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        if let Ok(s) = std::fs::read_to_string(path) {
            if let Ok(id) = Uuid::parse_str(s.trim()) {
                return Ok(Self::with_instance(id, header));
            }
        }
        let id = Uuid::new_v4();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, id.to_string())?;
        Ok(Self::with_instance(id, header))
    }

    /// The canonical id string. A UUID is *structured machine data*, so it
    /// travels as a routing key (`from`/`to`) and inside **JSON bodies** — never
    /// as a UNL word (UNL carries human/semantic content like services). Simple
    /// (hyphenless) form for compactness.
    pub fn id(&self) -> String {
        self.instance.simple().to_string()
    }

    /// A short label for logs: the friendly name if any, else a short id.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id()[..8].to_string())
    }
}

/// uuid ⇄ friendly-name aliases — display only, never the identity.
#[derive(Default)]
pub struct Aliases {
    by_name: HashMap<String, String>,
    by_id: HashMap<String, String>,
}

impl Aliases {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, name: impl Into<String>, id: impl Into<String>) {
        let (name, id) = (name.into(), id.into());
        self.by_name.insert(name.clone(), id.clone());
        self.by_id.insert(id, name);
    }

    pub fn id_of(&self, name: &str) -> Option<&str> {
        self.by_name.get(name).map(String::as_str)
    }

    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).map(String::as_str)
    }

    /// Render an id for humans: its friendly name if known, else the id itself.
    pub fn label<'a>(&'a self, id: &'a str) -> &'a str {
        self.name_of(id).unwrap_or(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn header() -> Header {
        Header {
            type_id: Uuid::parse_str("00000000-0000-0000-0000-0000000000aa").unwrap(),
            desc: "book-selling service".into(),
            name: Some("bookSeller".into()),
        }
    }

    fn unique_path() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!("aid-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
    }

    #[test]
    fn spawn_mints_a_fresh_unique_instance() {
        let a = AgentId::spawn(&header());
        let b = AgentId::spawn(&header());
        assert_ne!(a.instance, b.instance); // ephemeral: each spawn is distinct
        assert_eq!(a.agent_type.desc, "book-selling service");
        assert_eq!(a.label(), "bookSeller");
    }

    #[test]
    fn persistent_id_is_stable_across_restart() {
        let path = unique_path();
        let first = AgentId::load_or_mint(&header(), &path).unwrap();
        let again = AgentId::load_or_mint(&header(), &path).unwrap(); // "restart"
        assert_eq!(first.instance, again.instance); // persisted → stable
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn header_block_roundtrips() {
        let h = header();
        let back = Header::from_block(&h.to_block()).unwrap();
        assert_eq!(back.type_id, h.type_id);
        assert_eq!(back.name.as_deref(), Some("bookSeller"));
    }

    #[test]
    fn aliases_render_uuids_readably() {
        let seller = AgentId::spawn(&header());
        let mut a = Aliases::new();
        a.bind("bookSeller", seller.id());
        assert_eq!(a.id_of("bookSeller"), Some(seller.id().as_str()));
        assert_eq!(a.name_of(&seller.id()), Some("bookSeller"));
        assert_eq!(a.label(&seller.id()), "bookSeller");
        assert_eq!(a.label("unknown-uuid"), "unknown-uuid"); // falls back to the id
    }
}
