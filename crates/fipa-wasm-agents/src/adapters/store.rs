//! Concrete [`StateStore`] impls. [`SledStore`] is the normal-profile durable KV.
//!
//! Keys are **confined to the agent's namespace** (`THREAT_MODEL.md` R8): the stored
//! key is `len(ns) ‖ ns ‖ key`, so no crafted `key` can reach another agent's
//! namespace (the length prefix makes the boundary unforgeable).

use std::path::Path;

use anyhow::Result;

use super::StateStore;

/// A sled-backed, agent-namespaced key-value store.
pub struct SledStore {
    db: sled::Db,
}

impl SledStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(SledStore { db: sled::open(path)? })
    }

    /// `len(ns) ‖ ns ‖ key` — an unforgeable namespace prefix (R8).
    fn scoped(ns: &str, key: &str) -> Vec<u8> {
        let mut k = Vec::with_capacity(4 + ns.len() + key.len());
        k.extend_from_slice(&(ns.len() as u32).to_be_bytes());
        k.extend_from_slice(ns.as_bytes());
        k.extend_from_slice(key.as_bytes());
        k
    }
}

impl StateStore for SledStore {
    fn get(&self, ns: &str, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.db.get(Self::scoped(ns, key))?.map(|v| v.to_vec()))
    }
    fn put(&self, ns: &str, key: &str, val: &[u8]) -> Result<()> {
        self.db.insert(Self::scoped(ns, key), val)?;
        self.db.flush()?;
        Ok(())
    }
    fn del(&self, ns: &str, key: &str) -> Result<()> {
        self.db.remove(Self::scoped(ns, key))?;
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> SledStore {
        SledStore { db: sled::Config::new().temporary(true).open().unwrap() }
    }

    #[test]
    fn namespaced_get_put_del() {
        let s = temp_store();
        s.put("agentA", "k", b"1").unwrap();
        assert_eq!(s.get("agentA", "k").unwrap(), Some(b"1".to_vec()));
        // a different namespace cannot see it
        assert_eq!(s.get("agentB", "k").unwrap(), None);
        s.del("agentA", "k").unwrap();
        assert_eq!(s.get("agentA", "k").unwrap(), None);
    }

    #[test]
    fn namespace_boundary_cannot_be_escaped() {
        let s = temp_store();
        s.put("a", "x", b"secret").unwrap();
        // (ns="a", key="x") must not collide with (ns="", key="ax") — the length
        // prefix keeps the boundary unforgeable (R8).
        assert_eq!(s.get("", "ax").unwrap(), None);
        assert_eq!(s.get("ax", "").unwrap(), None);
    }
}
