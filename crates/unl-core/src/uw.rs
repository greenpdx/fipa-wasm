//! Universal Words (spec §2): the nodes of the hypergraph and their identities.

use crate::attr::AttrList;
use crate::graph::{Lang, NodeId, ScopeId};
use crate::relation::RelationTag;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// A Uniform Concept Identifier — the identity of a Universal Word.
///
/// Per the UNL 2010 spec §2.3, a concept is identified either by its position in
/// a knowledge base (UCL) or by a human-readable name (UCN).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Uci {
    /// Uniform Concept Locator: `ucl://<authority>/<id>`. In documents the
    /// authority is usually elided and only the id remains.
    Ucl {
        /// `None` => inferred from the document header.
        authority: Option<SmolStr>,
        /// 9-digit canonical, but stored full width.
        id: u64,
    },
    /// Uniform Concept Name: `ucn:<lang>:<root><suffix>`, e.g. `cat(icl>feline)`.
    Ucn {
        /// ISO 639-3; `None` => inferred.
        lang: Option<Lang>,
        root: SmolStr,
        suffix: Option<UcnSuffix>,
    },
    /// Temporary UW — a quoted, non-dictionary entity (`"UNDL Foundation"`, `"H2O"`).
    Temporary(SmolStr),
    /// Null / pro-UW — the `"00"` used for exophora, ellipsis, interjections.
    Null,
}

impl Uci {
    /// A bare UCL with no explicit authority (the common in-document form).
    pub const fn ucl(id: u64) -> Self {
        Uci::Ucl { authority: None, id }
    }

    /// A simple UCN root with no language and no disambiguating suffix.
    pub fn ucn(root: impl Into<SmolStr>) -> Self {
        Uci::Ucn {
            lang: None,
            root: root.into(),
            suffix: None,
        }
    }
}

/// The disambiguating suffix of a UCN, e.g. `(icl>feline)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UcnSuffix {
    /// `icl`, `iof`, `equ`, …
    pub relation: RelationTag,
    pub word: SmolStr,
}

/// A Universal Word as it appears in a graph: an identity plus annotations plus
/// the local node id used to wire relations together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uw {
    pub uci: Uci,
    pub attributes: AttrList,
    /// Node id local to the sentence (the `:01`, `:02`, `73`, `92` in the corpus).
    pub node_id: Option<NodeId>,
    /// Scope id for hyper-nodes / sub-graphs (`:01` in `and:01(...)`).
    pub scope: Option<ScopeId>,
}

impl Uw {
    /// A UW carrying just an identity — no attributes, no wiring yet.
    pub fn new(uci: Uci) -> Self {
        Uw {
            uci,
            attributes: AttrList::default(),
            node_id: None,
            scope: None,
        }
    }

    /// Builder: assign the local node id.
    pub fn with_node_id(mut self, id: impl Into<NodeId>) -> Self {
        self.node_id = Some(id.into());
        self
    }
}

/// Lexical category of a UW (spec §2.4) — semantic, not grammatical.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LexCategory {
    Nominal,    // N
    Verbal,     // V
    Adjectival, // J
    Adverbial,  // A
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ucl_helper_elides_authority() {
        assert_eq!(Uci::ucl(102121620), Uci::Ucl { authority: None, id: 102121620 });
    }

    #[test]
    fn uci_is_hashable() {
        let mut set = std::collections::HashSet::new();
        set.insert(Uci::ucl(1));
        set.insert(Uci::ucn("cat"));
        set.insert(Uci::Null);
        assert_eq!(set.len(), 3);
        assert!(set.contains(&Uci::ucl(1)));
    }

    #[test]
    fn uw_builder() {
        let uw = Uw::new(Uci::ucn("cat")).with_node_id("01");
        assert_eq!(uw.node_id, Some(NodeId::from("01")));
        assert!(uw.attributes.is_empty());
    }
}
