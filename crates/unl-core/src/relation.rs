//! Universal relations (spec §4): the directed, binary, labelled arcs of the
//! hypergraph, plus the subsumption hierarchy that lets an agent fall back from
//! a specific relation to a more general one it does understand.

use crate::error::CoreError;
use crate::graph::{NodeId, ScopeId};
use crate::uw::Uw;
use serde::{Deserialize, Serialize};

/// The closed set of universal relation tags (spec §4.4).
///
/// The variants are the UNL 2010 alphabetical list. Subsumption between tags is
/// expressed by [`RelationTag::parent`]; the data mirrors `data/relations.toml`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationTag {
    Agt, And, Ant, Aoj, Bas, Ben, Cnt, Con, Coo, Dur, Equ, Exp, Fld,
    Gol, Icl, Ins, Iof, Lpl, Man, Mat, Met, Mod, Nam, Obj, Opl, Or,
    Per, Plc, Pof, Pos, Ptn, Pur, Qua, Res, Rsn, Seq, Src, Tim, Tmf, Tmt, Via,
}

impl RelationTag {
    /// Every tag, in declaration (alphabetical) order. Handy for table-driven
    /// code and tests.
    pub const ALL: [RelationTag; 41] = {
        use RelationTag::*;
        [
            Agt, And, Ant, Aoj, Bas, Ben, Cnt, Con, Coo, Dur, Equ, Exp, Fld, Gol, Icl, Ins, Iof,
            Lpl, Man, Mat, Met, Mod, Nam, Obj, Opl, Or, Per, Plc, Pof, Pos, Ptn, Pur, Qua, Res,
            Rsn, Seq, Src, Tim, Tmf, Tmt, Via,
        ]
    };

    /// The canonical three-letter (or two-letter) lowercase tag as it appears in
    /// UNL text, e.g. `RelationTag::Agt.as_str() == "agt"`.
    pub const fn as_str(self) -> &'static str {
        use RelationTag::*;
        match self {
            Agt => "agt", And => "and", Ant => "ant", Aoj => "aoj", Bas => "bas", Ben => "ben",
            Cnt => "cnt", Con => "con", Coo => "coo", Dur => "dur", Equ => "equ", Exp => "exp",
            Fld => "fld", Gol => "gol", Icl => "icl", Ins => "ins", Iof => "iof", Lpl => "lpl",
            Man => "man", Mat => "mat", Met => "met", Mod => "mod", Nam => "nam", Obj => "obj",
            Opl => "opl", Or => "or", Per => "per", Plc => "plc", Pof => "pof", Pos => "pos",
            Ptn => "ptn", Pur => "pur", Qua => "qua", Res => "res", Rsn => "rsn", Seq => "seq",
            Src => "src", Tim => "tim", Tmf => "tmf", Tmt => "tmt", Via => "via",
        }
    }

    /// The immediate parent in the subsumption hierarchy (spec §4.3), if any.
    ///
    /// The place family (`gol`, `lpl`, `src`, `via`) refines `plc`; the
    /// boundary-time relations (`tmf`, `tmt`) refine `tim`. The remaining tags
    /// are roots in Rev 1; more edges are added here (and in
    /// `data/relations.toml`) as the hierarchy is transcribed.
    pub const fn parent(self) -> Option<RelationTag> {
        use RelationTag::*;
        match self {
            Gol | Lpl | Src | Via => Some(Plc),
            Tmf | Tmt => Some(Tim),
            _ => None,
        }
    }

    /// Walk up the hierarchy, yielding `self` first, then each ancestor up to the
    /// root. Always finite (the hierarchy is a forest).
    pub fn ancestors(self) -> impl Iterator<Item = RelationTag> {
        let mut next = Some(self);
        std::iter::from_fn(move || {
            let cur = next?;
            next = cur.parent();
            Some(cur)
        })
    }

    /// True if `self` *is* `other` or a refinement of it. `Gol.is_a(Plc)` is
    /// true; `Plc.is_a(Gol)` is false; `Agt.is_a(Agt)` is true (reflexive).
    pub fn is_a(self, other: RelationTag) -> bool {
        self.ancestors().any(|t| t == other)
    }
}

impl std::str::FromStr for RelationTag {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RelationTag::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| CoreError::UnknownRelation(s.to_string()))
    }
}

impl std::fmt::Display for RelationTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A relation argument: either a reference to a node declared elsewhere (list
/// format) or an inline UW (table format).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeRef {
    /// `"01"`, `"73"` — points at a [`Uw`] declared in `[W]...[/W]`.
    Id(NodeId),
    /// `":01"` — references a sub-graph / hyper-node.
    Scope(ScopeId),
    /// Table format: the UW is written in place.
    Inline(Box<Uw>),
}

impl NodeRef {
    /// True when this argument is a plain reference to the given node id.
    pub fn is_node(&self, id: &NodeId) -> bool {
        matches!(self, NodeRef::Id(n) if n == id)
    }
}

/// A binary relation instance in a graph: `<tag>:<scope>(<source>, <target>)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub tag: RelationTag,
    pub scope: Option<ScopeId>,
    pub source: NodeRef,
    pub target: NodeRef,
}

impl Relation {
    /// Construct a relation between two declared nodes (the common list-format case).
    pub fn between(tag: RelationTag, source: NodeId, target: NodeId) -> Self {
        Relation {
            tag,
            scope: None,
            source: NodeRef::Id(source),
            target: NodeRef::Id(target),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_complete_and_unique() {
        assert_eq!(RelationTag::ALL.len(), 41);
        let mut seen = std::collections::HashSet::new();
        for t in RelationTag::ALL {
            assert!(seen.insert(t.as_str()), "duplicate tag {}", t.as_str());
        }
    }

    #[test]
    fn str_roundtrips_for_every_tag() {
        for t in RelationTag::ALL {
            assert_eq!(t.as_str().parse::<RelationTag>().unwrap(), t);
        }
    }

    #[test]
    fn unknown_tag_errors() {
        assert!("xyz".parse::<RelationTag>().is_err());
    }

    #[test]
    fn place_family_refines_plc() {
        for t in [RelationTag::Gol, RelationTag::Lpl, RelationTag::Src, RelationTag::Via] {
            assert_eq!(t.parent(), Some(RelationTag::Plc));
            assert!(t.is_a(RelationTag::Plc), "{} should be a plc", t);
        }
        assert!(!RelationTag::Plc.is_a(RelationTag::Gol));
    }

    #[test]
    fn boundary_times_refine_tim() {
        assert!(RelationTag::Tmf.is_a(RelationTag::Tim));
        assert!(RelationTag::Tmt.is_a(RelationTag::Tim));
        assert_eq!(RelationTag::Dur.parent(), None); // duration is a root in Rev 1
    }

    #[test]
    fn is_a_is_reflexive() {
        for t in RelationTag::ALL {
            assert!(t.is_a(t));
        }
    }

    #[test]
    fn ancestors_terminate_at_a_root() {
        let chain: Vec<_> = RelationTag::Gol.ancestors().collect();
        assert_eq!(chain, vec![RelationTag::Gol, RelationTag::Plc]);
    }
}
