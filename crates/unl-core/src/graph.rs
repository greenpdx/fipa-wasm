//! The graph and document model (spec §6), plus the small id newtypes.

use crate::error::CoreError;
use crate::relation::Relation;
use crate::uw::Uw;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt;

/// Local node id within a sentence: `"01"`, `"73"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub SmolStr);

/// Scope id for a sub-graph / hyper-node: `":01"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopeId(pub SmolStr);

impl<T: Into<SmolStr>> From<T> for NodeId {
    fn from(s: T) -> Self {
        NodeId(s.into())
    }
}

impl<T: Into<SmolStr>> From<T> for ScopeId {
    fn from(s: T) -> Self {
        ScopeId(s.into())
    }
}

/// An ISO 639-3 language code, e.g. `eng`. Stored as three ASCII lowercase
/// bytes; serializes as the three-letter string (not a byte array).
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Lang([u8; 3]);

impl Lang {
    pub const ENG: Lang = Lang(*b"eng");
    pub const SPA: Lang = Lang(*b"spa");
    pub const FRA: Lang = Lang(*b"fra");

    /// Validate and construct from a 3-letter code. Rejects anything that is not
    /// exactly three ASCII lowercase letters.
    pub fn new(code: &str) -> Result<Self, CoreError> {
        let b = code.as_bytes();
        if b.len() != 3 || !b.iter().all(u8::is_ascii_lowercase) {
            return Err(CoreError::InvalidLang(code.to_string()));
        }
        Ok(Lang([b[0], b[1], b[2]]))
    }

    pub fn as_str(&self) -> &str {
        // Safe: only constructed from ASCII lowercase bytes.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Debug for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lang({:?})", self.as_str())
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Lang {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let code = SmolStr::deserialize(d)?;
        Lang::new(&code).map_err(serde::de::Error::custom)
    }
}

/// A single UNL sentence: a semantic hypergraph. The fundamental unit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnlGraph {
    /// All UWs, keyed by their local node id (list format declares these
    /// explicitly; table format populates this by hoisting inline UWs).
    pub nodes: IndexMap<NodeId, Uw>,
    /// All relation arcs.
    pub relations: Vec<Relation>,
    /// The entry node, if marked (`@entry`) — the semantic head.
    pub entry: Option<NodeId>,
}

impl UnlGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a UW under a node id, returning the id for convenience.
    pub fn insert_node(&mut self, id: impl Into<NodeId>, uw: Uw) -> NodeId {
        let id = id.into();
        self.nodes.insert(id.clone(), uw);
        id
    }

    /// Add a relation arc.
    pub fn add_relation(&mut self, rel: Relation) {
        self.relations.push(rel);
    }

    /// True if every plain node reference in every relation resolves to a
    /// declared node. (Inline and scope references are not checked here.)
    pub fn refs_resolve(&self) -> bool {
        self.relations.iter().all(|r| {
            self.ref_ok(&r.source) && self.ref_ok(&r.target)
        })
    }

    fn ref_ok(&self, r: &crate::relation::NodeRef) -> bool {
        match r {
            crate::relation::NodeRef::Id(id) => self.nodes.contains_key(id),
            // Scope/inline references are resolved by later layers.
            _ => true,
        }
    }
}

/// A UNL document: an ordered collection of sentences with provenance. Mirrors
/// the UNL/XML document structure (spec §6).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnlDocument {
    pub metadata: DocMetadata,
    pub sentences: Vec<UnlSentence>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnlSentence {
    pub id: Option<SmolStr>,
    /// Original natural-language sources, keyed by language (`<unl:org>`).
    pub org: Vec<(Lang, String)>,
    pub graph: UnlGraph,
    /// Generated outputs, keyed by language (`<unl:out>`).
    pub out: Vec<(Lang, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocMetadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub date: Option<String>,
    pub language: Option<Lang>,
    pub scheme: Option<String>,    // "UNL 2010"
    pub authority: Option<String>, // "https://kb.crmep.com"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_validates() {
        assert_eq!(Lang::new("eng").unwrap(), Lang::ENG);
        assert!(Lang::new("EN").is_err());
        assert!(Lang::new("english").is_err());
        assert!(Lang::new("e1g").is_err());
    }

    #[test]
    fn lang_serializes_as_string() {
        let j = serde_json::to_string(&Lang::ENG).unwrap();
        assert_eq!(j, "\"eng\"");
        let back: Lang = serde_json::from_str(&j).unwrap();
        assert_eq!(back, Lang::ENG);
    }

    #[test]
    fn dangling_reference_detected() {
        use crate::relation::{Relation, RelationTag};
        use crate::uw::{Uci, Uw};

        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("kill")));
        // "02" was never declared.
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
        assert!(!g.refs_resolve());

        g.insert_node("02", Uw::new(Uci::ucn("Peter")));
        assert!(g.refs_resolve());
    }
}
