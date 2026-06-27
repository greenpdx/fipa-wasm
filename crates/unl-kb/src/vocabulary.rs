//! [`Vocabulary`] — an agent's compact, self-defined UNL vocabulary.
//!
//! Edge / IoT agents do not carry the central KB (WordNet/SledKb). They carry a
//! *vocabulary*: the concepts, relations, and attributes they understand. To
//! keep it cheap on constrained devices it is just what the agent needs — but
//! because **the agent defines it**, it can be as small or as large as the job
//! requires (manifest §7, LoRaWAN/edge note).
//!
//! Each concept may carry minimal local structure (`icl`/`iof` parents) so the
//! agent can do *limited* reasoning on-device — hence `Vocabulary` implements
//! [`KnowledgeBase`]. Beyond concepts it also whitelists the [`RelationTag`]s
//! and [`Attr`]s the agent accepts.
//!
//! Verification at the edge (in `unl-validator`) means "is this message within
//! my vocabulary and structurally sound?" — any out-of-vocabulary term is a
//! `not-understood`, by construction.

use crate::{ConceptFeatures, KbError, KnowledgeBase};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};
use unl_core::{Attr, Lang, NodeRef, Relation, RelationTag, Uci, Uw};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VocabConcept {
    features: ConceptFeatures,
    #[serde(default)]
    icl: Vec<u64>,
    #[serde(default)]
    iof: Vec<u64>,
}

/// An agent's compact, self-defined UNL vocabulary. Serializable for shipping to
/// a device.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Vocabulary {
    concepts: HashMap<u64, VocabConcept>,
    #[serde(default)]
    by_lemma: HashMap<SmolStr, Vec<u64>>,
    relations: HashSet<RelationTag>,
    attributes: HashSet<Attr>,
}

impl Vocabulary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a concept the agent understands, with optional local `icl`/`iof`
    /// structure (parents that should themselves be in the vocabulary) and
    /// lemmas for resolution.
    pub fn allow_concept(
        &mut self,
        id: u64,
        features: ConceptFeatures,
        icl: Vec<u64>,
        iof: Vec<u64>,
        lemmas: &[&str],
    ) {
        for lemma in lemmas {
            self.by_lemma.entry(SmolStr::new(lemma)).or_default().push(id);
        }
        self.concepts.insert(id, VocabConcept { features, icl, iof });
    }

    /// Builder form of [`Self::allow_concept`].
    pub fn with_concept(
        mut self,
        id: u64,
        features: ConceptFeatures,
        icl: Vec<u64>,
        iof: Vec<u64>,
        lemmas: &[&str],
    ) -> Self {
        self.allow_concept(id, features, icl, iof, lemmas);
        self
    }

    pub fn allow_relation(&mut self, tag: RelationTag) {
        self.relations.insert(tag);
    }

    pub fn allow_relations(&mut self, tags: impl IntoIterator<Item = RelationTag>) {
        self.relations.extend(tags);
    }

    pub fn allow_attribute(&mut self, attr: Attr) {
        self.attributes.insert(attr);
    }

    pub fn allow_attributes(&mut self, attrs: impl IntoIterator<Item = Attr>) {
        self.attributes.extend(attrs);
    }

    /// Carve a vocabulary out of a larger knowledge base: pull `concept_ids`
    /// (with their features and the subset of `icl`/`iof` parents that are *also*
    /// in `concept_ids`), and whitelist the given relations and attributes.
    pub fn extract(
        kb: &dyn KnowledgeBase,
        concept_ids: &[u64],
        relations: impl IntoIterator<Item = RelationTag>,
        attributes: impl IntoIterator<Item = Attr>,
    ) -> Result<Self, KbError> {
        let in_vocab: HashSet<u64> = concept_ids.iter().copied().collect();
        let mut vocab = Vocabulary::new();
        vocab.allow_relations(relations);
        vocab.allow_attributes(attributes);
        for &id in concept_ids {
            let uci = Uci::ucl(id);
            let Some(features) = kb.features(&uci)? else {
                continue;
            };
            let mut icl = Vec::new();
            let mut iof = Vec::new();
            for rel in kb.definition(&uci)? {
                if let NodeRef::Inline(uw) = &rel.target
                    && let Uci::Ucl { id: parent, .. } = uw.uci
                    && in_vocab.contains(&parent)
                {
                    match rel.tag {
                        RelationTag::Icl => icl.push(parent),
                        RelationTag::Iof => iof.push(parent),
                        _ => {}
                    }
                }
            }
            vocab.concepts.insert(id, VocabConcept { features, icl, iof });
        }
        Ok(vocab)
    }

    /// True if the relation tag is in the agent's vocabulary.
    pub fn allows_relation(&self, tag: RelationTag) -> bool {
        self.relations.contains(&tag)
    }

    /// True if the attribute is in the agent's vocabulary.
    pub fn allows_attribute(&self, attr: &Attr) -> bool {
        self.attributes.contains(attr)
    }

    /// True if the concept identity is in the agent's vocabulary.
    pub fn knows(&self, concept: &Uci) -> bool {
        self.id_of(concept).is_some()
    }

    pub fn concept_count(&self) -> usize {
        self.concepts.len()
    }

    pub fn relation_count(&self) -> usize {
        self.relations.len()
    }

    pub fn attribute_count(&self) -> usize {
        self.attributes.len()
    }

    fn id_of(&self, u: &Uci) -> Option<u64> {
        match u {
            Uci::Ucl { id, .. } => self.concepts.contains_key(id).then_some(*id),
            Uci::Ucn { root, .. } => self.by_lemma.get(root).and_then(|v| v.first()).copied(),
            _ => None,
        }
    }
}

impl KnowledgeBase for Vocabulary {
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError> {
        Ok(self.id_of(ucn).map(Uci::ucl))
    }

    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError> {
        let id = self.id_of(concept).ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let c = &self.concepts[&id];
        let source = Uci::ucl(id);
        let rel = |tag, target| Relation {
            tag,
            scope: None,
            source: NodeRef::Inline(Box::new(Uw::new(source.clone()))),
            target: NodeRef::Inline(Box::new(Uw::new(Uci::ucl(target)))),
        };
        Ok(c.icl
            .iter()
            .map(|&t| rel(RelationTag::Icl, t))
            .chain(c.iof.iter().map(|&t| rel(RelationTag::Iof, t)))
            .collect())
    }

    fn features(&self, concept: &Uci) -> Result<Option<ConceptFeatures>, KbError> {
        Ok(self.id_of(concept).map(|id| self.concepts[&id].features.clone()))
    }

    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError> {
        let (Some(start), Some(goal)) = (self.id_of(sub), self.id_of(sup)) else {
            return Ok(false);
        };
        if start == goal {
            return Ok(true);
        }
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            let Some(c) = self.concepts.get(&cur) else { continue };
            for &parent in c.icl.iter().chain(c.iof.iter()) {
                if parent == goal {
                    return Ok(true);
                }
                if seen.insert(parent) {
                    stack.push(parent);
                }
            }
        }
        Ok(false)
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
        let Some(c) = self.concepts.get(&src) else { return Ok(0) };
        let holds = match tag {
            RelationTag::Icl => c.icl.contains(&tgt),
            RelationTag::Iof => c.iof.contains(&tgt),
            _ => false,
        };
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
    use unl_core::LexCategory;

    fn feat() -> ConceptFeatures {
        ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None }
    }

    /// A tiny sensor vocabulary: vehicle(1) iof car(2); relations agt/obj/plc;
    /// attributes def/past.
    fn sensor_vocab() -> Vocabulary {
        let mut v = Vocabulary::new();
        v.allow_concept(2, feat(), vec![], vec![], &["car"]);
        v.allow_concept(1, feat(), vec![], vec![2], &["vehicle"]); // vehicle iof car
        v.allow_relations([RelationTag::Agt, RelationTag::Obj, RelationTag::Plc]);
        v.allow_attributes([Attr::Def, Attr::Past]);
        v
    }

    #[test]
    fn membership() {
        let v = sensor_vocab();
        assert!(v.knows(&Uci::ucl(1)));
        assert!(v.knows(&Uci::ucn("vehicle")));
        assert!(!v.knows(&Uci::ucl(999)));
        assert!(v.allows_relation(RelationTag::Agt));
        assert!(!v.allows_relation(RelationTag::Icl));
        assert!(v.allows_attribute(&Attr::Def));
        assert!(!v.allows_attribute(&Attr::Plural));
        assert_eq!((v.concept_count(), v.relation_count(), v.attribute_count()), (2, 3, 2));
    }

    #[test]
    fn limited_local_reasoning() {
        let v = sensor_vocab();
        // The agent can reason within its vocabulary: vehicle iof car.
        assert!(v.is_a(&Uci::ucn("vehicle"), &Uci::ucn("car")).unwrap());
        assert_eq!(v.resolve(&Uci::ucn("vehicle")).unwrap(), Some(Uci::ucl(1)));
        assert!(v.definition(&Uci::ucl(1)).unwrap().iter().any(|r| r.tag == RelationTag::Iof));
        let dynkb: &dyn KnowledgeBase = &v;
        assert!(dynkb.features(&Uci::ucl(1)).unwrap().is_some());
    }

    #[test]
    fn extract_from_kb_keeps_only_in_vocab_parents() {
        use crate::MemKb;
        // cat -> feline -> ... -> animal in the fixture; extract just cat+feline.
        let kb = MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap();
        let cat = 102121620;
        let feline = 102120000;
        let vocab = Vocabulary::extract(&kb, &[cat, feline], [RelationTag::Icl], [Attr::Def]).unwrap();
        assert!(vocab.knows(&Uci::ucl(cat)));
        // cat icl feline survives (feline is in vocab); the rest of the chain is dropped.
        assert!(vocab.is_a(&Uci::ucl(cat), &Uci::ucl(feline)).unwrap());
        assert!(!vocab.knows(&Uci::ucl(100015388))); // animal not extracted
        assert!(vocab.allows_relation(RelationTag::Icl));
    }

    #[test]
    fn serde_roundtrip() {
        let v = sensor_vocab();
        let json = serde_json::to_string(&v).unwrap();
        let back: Vocabulary = serde_json::from_str(&json).unwrap();
        assert!(back.knows(&Uci::ucn("vehicle")));
        assert!(back.allows_relation(RelationTag::Plc));
        assert!(back.allows_attribute(&Attr::Past));
    }
}
