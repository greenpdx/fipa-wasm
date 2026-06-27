//! [`GraphKb`] — a graph-structured knowledge base.
//!
//! The KV-backed stores ([`crate::MemKb`], [`crate::SledKb`]) answer point
//! lookups and walk *up* the `icl`/`iof` chain. A graph model adds the
//! capability they lack cheaply: **reverse traversal** (descendants — the
//! subtypes/instances of a concept) and **path queries**, by keeping both
//! forward and reverse adjacency.
//!
//! This impl is in-memory (a typed directed multigraph). It is the graph-model
//! shape of the [`KnowledgeBase`] role; a persistent graph database (Neo4j,
//! oxigraph, …) is a drop-in behind the same trait — exactly the substitution
//! point the manifest's open-core boundary anticipates.

use crate::{ConceptFeatures, KbError, KnowledgeBase, WordNetKb};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet, VecDeque};
use unl_core::{Lang, NodeRef, Relation, RelationTag, Uci, Uw};

struct Node {
    features: ConceptFeatures,
    /// Outgoing edges: (relation, target id).
    out: Vec<(RelationTag, u64)>,
}

/// In-memory, graph-structured knowledge base.
#[derive(Default)]
pub struct GraphKb {
    nodes: HashMap<u64, Node>,
    /// Reverse adjacency: target id -> (relation, source id).
    incoming: HashMap<u64, Vec<(RelationTag, u64)>>,
    /// Lemma -> concept ids (language-agnostic in Rev 1).
    by_lemma: HashMap<SmolStr, Vec<u64>>,
}

impl GraphKb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a concept and its outgoing edges; maintains the reverse index.
    pub fn add_concept(
        &mut self,
        id: u64,
        features: ConceptFeatures,
        links: Vec<(RelationTag, u64)>,
        lemmas: &[&str],
    ) {
        for &(tag, target) in &links {
            self.incoming.entry(target).or_default().push((tag, id));
        }
        for lemma in lemmas {
            self.by_lemma.entry(SmolStr::new(lemma)).or_default().push(id);
        }
        self.nodes.insert(id, Node { features, out: links });
    }

    /// Load a whole graph KB from a WordNet seed (in memory).
    pub fn from_wordnet(wordnet: &WordNetKb) -> Result<Self, KbError> {
        let mut kb = GraphKb::new();
        wordnet.for_each_concept(|id, features, links| {
            kb.add_concept(id, features, links, &[]);
        })?;
        for (lemma, ucls) in wordnet.index_entries() {
            let entry = kb.by_lemma.entry(SmolStr::new(lemma)).or_default();
            entry.extend(ucls);
        }
        Ok(kb)
    }

    fn id_of(&self, u: &Uci) -> Option<u64> {
        match u {
            Uci::Ucl { id, .. } => self.nodes.contains_key(id).then_some(*id),
            Uci::Ucn { root, .. } => self.by_lemma.get(root).and_then(|v| v.first()).copied(),
            _ => None,
        }
    }

    /// All `icl`/`iof` ancestors (supertypes/classes), reachable upward.
    pub fn ancestors(&self, id: u64) -> Vec<u64> {
        self.reachable(id, true)
    }

    /// All `icl`/`iof` descendants (subtypes/instances), reachable downward via
    /// the reverse index. The KV stores cannot answer this without a full scan.
    pub fn descendants(&self, id: u64) -> Vec<u64> {
        self.reachable(id, false)
    }

    fn reachable(&self, start: u64, up: bool) -> Vec<u64> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut queue = VecDeque::from([start]);
        while let Some(cur) = queue.pop_front() {
            let edges = if up {
                self.nodes.get(&cur).map(|n| n.out.as_slice())
            } else {
                self.incoming.get(&cur).map(|v| v.as_slice())
            };
            for &(tag, other) in edges.unwrap_or(&[]) {
                if matches!(tag, RelationTag::Icl | RelationTag::Iof) && seen.insert(other) {
                    out.push(other);
                    queue.push_back(other);
                }
            }
        }
        out
    }

    /// Shortest `icl`/`iof` path from `from` up to `to` (inclusive of both),
    /// if `to` is an ancestor of `from`.
    pub fn path(&self, from: u64, to: u64) -> Option<Vec<u64>> {
        if from == to {
            return Some(vec![from]);
        }
        let mut prev: HashMap<u64, u64> = HashMap::new();
        let mut queue = VecDeque::from([from]);
        let mut seen = HashSet::from([from]);
        while let Some(cur) = queue.pop_front() {
            let Some(node) = self.nodes.get(&cur) else { continue };
            for &(tag, parent) in &node.out {
                if !matches!(tag, RelationTag::Icl | RelationTag::Iof) || !seen.insert(parent) {
                    continue;
                }
                prev.insert(parent, cur);
                if parent == to {
                    // reconstruct
                    let mut path = vec![to];
                    let mut step = to;
                    while let Some(&p) = prev.get(&step) {
                        path.push(p);
                        step = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(parent);
            }
        }
        None
    }
}

impl KnowledgeBase for GraphKb {
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError> {
        Ok(self.id_of(ucn).map(Uci::ucl))
    }

    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError> {
        let id = self.id_of(concept).ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let node = &self.nodes[&id];
        let source = Uci::ucl(id);
        Ok(node
            .out
            .iter()
            .map(|&(tag, target)| Relation {
                tag,
                scope: None,
                source: NodeRef::Inline(Box::new(Uw::new(source.clone()))),
                target: NodeRef::Inline(Box::new(Uw::new(Uci::ucl(target)))),
            })
            .collect())
    }

    fn features(&self, concept: &Uci) -> Result<Option<ConceptFeatures>, KbError> {
        Ok(self.id_of(concept).map(|id| self.nodes[&id].features.clone()))
    }

    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError> {
        let (Some(a), Some(b)) = (self.id_of(sub), self.id_of(sup)) else {
            return Ok(false);
        };
        Ok(a == b || self.ancestors(a).contains(&b))
    }

    fn relation_certainty(
        &self,
        tag: RelationTag,
        source: &Uci,
        target: &Uci,
    ) -> Result<u8, KbError> {
        let (Some(src), Some(tgt)) = (self.id_of(source), self.id_of(target)) else {
            return Ok(0);
        };
        let holds = self
            .nodes
            .get(&src)
            .is_some_and(|n| n.out.iter().any(|&(t, target)| t == tag && target == tgt));
        Ok(if holds { 255 } else { 0 })
    }

    fn candidates(&self, lemma: &str, _lang: Lang) -> Result<Vec<Uci>, KbError> {
        Ok(self
            .by_lemma
            .get(lemma)
            .map(|ids| ids.iter().copied().map(Uci::ucl).collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use unl_core::LexCategory;

    fn feat() -> ConceptFeatures {
        ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None }
    }

    /// cat(1) -> feline(2) -> carnivore(3) -> animal(4); dog(5) -> carnivore(3).
    fn fixture() -> GraphKb {
        let mut kb = GraphKb::new();
        kb.add_concept(4, feat(), vec![], &["animal"]);
        kb.add_concept(3, feat(), vec![(RelationTag::Icl, 4)], &["carnivore"]);
        kb.add_concept(2, feat(), vec![(RelationTag::Icl, 3)], &["feline"]);
        kb.add_concept(1, feat(), vec![(RelationTag::Icl, 2)], &["cat"]);
        kb.add_concept(5, feat(), vec![(RelationTag::Icl, 3)], &["dog"]);
        kb
    }

    #[test]
    fn ancestors_walk_up() {
        let kb = fixture();
        assert_eq!(kb.ancestors(1), vec![2, 3, 4]);
        assert!(kb.ancestors(4).is_empty());
    }

    #[test]
    fn descendants_walk_down() {
        let kb = fixture();
        // Carnivore's subtypes: feline, cat, dog (order is BFS).
        let mut desc = kb.descendants(3);
        desc.sort();
        assert_eq!(desc, vec![1, 2, 5]);
    }

    #[test]
    fn shortest_path_up() {
        let kb = fixture();
        assert_eq!(kb.path(1, 4), Some(vec![1, 2, 3, 4]));
        assert_eq!(kb.path(1, 1), Some(vec![1]));
        assert_eq!(kb.path(4, 1), None); // not an ancestor
    }

    #[test]
    fn knowledge_base_surface() {
        let kb = fixture();
        assert_eq!(kb.resolve(&Uci::ucn("cat")).unwrap(), Some(Uci::ucl(1)));
        assert!(kb.is_a(&Uci::ucn("cat"), &Uci::ucn("animal")).unwrap());
        assert!(!kb.is_a(&Uci::ucn("animal"), &Uci::ucn("cat")).unwrap());
        assert_eq!(kb.definition(&Uci::ucl(1)).unwrap()[0].tag, RelationTag::Icl);
        assert_eq!(
            kb.relation_certainty(RelationTag::Icl, &Uci::ucl(1), &Uci::ucl(2)).unwrap(),
            255
        );
        assert_eq!(kb.candidates("dog", Lang::ENG).unwrap(), vec![Uci::ucl(5)]);
        let dynkb: &dyn KnowledgeBase = &kb;
        assert!(dynkb.features(&Uci::ucl(1)).unwrap().is_some());
    }

    /// Build a whole graph KB from the real WordNet seed and run a reverse query
    /// (descendants) that the KV stores can't answer directly. Heavy (loads
    /// ~117k concepts into memory) — run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "loads full WordNet into memory"]
    fn from_wordnet_reverse_query() {
        let dict = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/kb-seed/wordnet-3.1/dict");
        if !dict.join("data.noun").exists() {
            eprintln!("skip: WordNet not downloaded");
            return;
        }
        let wn = WordNetKb::open(&dict).unwrap();
        let kb = GraphKb::from_wordnet(&wn).unwrap();

        let cat = match kb.resolve(&Uci::ucn("cat")).unwrap().unwrap() {
            Uci::Ucl { id, .. } => id,
            _ => unreachable!(),
        };
        let feline = match kb.resolve(&Uci::ucn("feline")).unwrap().unwrap() {
            Uci::Ucl { id, .. } => id,
            _ => unreachable!(),
        };
        // cat is (transitively) a kind of feline => feline's descendants include cat.
        assert!(kb.descendants(feline).contains(&cat));
        assert!(kb.is_a(&Uci::ucl(cat), &Uci::ucl(feline)).unwrap());
    }
}
