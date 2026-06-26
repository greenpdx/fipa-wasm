//! The behavioural surface other crates implement against (spec §2.5).

use crate::graph::{NodeId, UnlGraph};
use crate::relation::Relation;
use crate::uw::Uw;

/// Anything that can be rendered to the canonical UNL text format. The
/// implementation for graphs/documents lives in `unl-parser`.
pub trait ToUnl {
    fn to_unl(&self, format: UnlFormat) -> String;
}

/// Format selector for serialization (spec §5).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UnlFormat {
    /// Relation-per-line: `aoj(300986027, 102121620.@def)`.
    Table,
    /// `[W]...[/W][R]...[/R]`.
    List,
}

/// A graph that can be walked for inference / traversal. Implemented by
/// [`UnlGraph`] and by KB-backed virtual graphs alike.
pub trait SemanticGraph {
    fn nodes(&self) -> impl Iterator<Item = &Uw>;
    fn relations(&self) -> impl Iterator<Item = &Relation>;
    /// All relations whose source is `node`.
    fn outgoing(&self, node: &NodeId) -> impl Iterator<Item = &Relation>;
    /// All relations whose target is `node`.
    fn incoming(&self, node: &NodeId) -> impl Iterator<Item = &Relation>;
}

impl SemanticGraph for UnlGraph {
    fn nodes(&self) -> impl Iterator<Item = &Uw> {
        self.nodes.values()
    }

    fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.iter()
    }

    fn outgoing(&self, node: &NodeId) -> impl Iterator<Item = &Relation> {
        self.relations.iter().filter(move |r| r.source.is_node(node))
    }

    fn incoming(&self, node: &NodeId) -> impl Iterator<Item = &Relation> {
        self.relations.iter().filter(move |r| r.target.is_node(node))
    }
}

/// Equality of meaning, not of surface form. Two graphs are unl-equivalent if
/// they normalize to the same canonical form ("Peter killed John" ==
/// "John was killed by Peter").
pub trait UnlEquivalent {
    fn unl_eq(&self, other: &Self) -> bool;
}

/// Cheap, KB-free structural equivalence: same entry, same nodes, and the same
/// *multiset* of relations (order-independent). This is the comparison the
/// manifest calls cheap once both graphs are normalized — semantic equivalence
/// is `a.normalize(..).unl_eq(&b.normalize(..))`, with normalization living in
/// `unl-validator`. (Implemented here because both the trait and `UnlGraph` are
/// local to this crate; an impl in `unl-validator` would violate the orphan
/// rule.)
impl UnlEquivalent for UnlGraph {
    fn unl_eq(&self, other: &Self) -> bool {
        self.entry == other.entry
            && self.nodes == other.nodes
            && relations_multiset_eq(&self.relations, &other.relations)
    }
}

fn relations_multiset_eq(a: &[Relation], b: &[Relation]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut used = vec![false; b.len()];
    'outer: for x in a {
        for (i, y) in b.iter().enumerate() {
            if !used[i] && x == y {
                used[i] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relation::{Relation, RelationTag};
    use crate::uw::{Uci, Uw};

    fn sample() -> UnlGraph {
        // agt(kill, Peter) ; obj(kill, John)
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("kill")));
        g.insert_node("02", Uw::new(Uci::ucn("Peter")));
        g.insert_node("03", Uw::new(Uci::ucn("John")));
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
        g.add_relation(Relation::between(RelationTag::Obj, "01".into(), "03".into()));
        g
    }

    #[test]
    fn outgoing_and_incoming() {
        let g = sample();
        let kill: NodeId = "01".into();
        assert_eq!(g.outgoing(&kill).count(), 2);
        assert_eq!(g.incoming(&kill).count(), 0);

        let peter: NodeId = "02".into();
        assert_eq!(g.incoming(&peter).count(), 1);
        assert_eq!(g.outgoing(&peter).count(), 0);
    }

    #[test]
    fn nodes_and_relations_iterate() {
        let g = sample();
        assert_eq!(SemanticGraph::nodes(&g).count(), 3);
        assert_eq!(SemanticGraph::relations(&g).count(), 2);
    }

    #[test]
    fn unl_eq_is_order_independent() {
        let a = sample();
        let mut b = sample();
        b.relations.reverse();
        assert!(a.unl_eq(&b));

        // Different relation set => not equivalent.
        let mut c = sample();
        c.relations.pop();
        assert!(!a.unl_eq(&c));
    }
}
