//! [`MemKb`] — an in-memory, `HashMap`-backed [`KnowledgeBase`] for tests and
//! development. Deterministic, no I/O, seeded either programmatically or from a
//! small TOML fixture (see `data/kb-seed/memkb-fixture.toml`).

use crate::{ConceptFeatures, KbError, KnowledgeBase};
use serde::Deserialize;
use smol_str::SmolStr;
use std::collections::HashMap;
use unl_core::{Lang, LexCategory, NodeRef, Relation, RelationTag, Uci, Uw};

/// A concept as supplied to [`MemKb`] — by TOML deserialization or by hand.
/// Ontology links (`icl`, `iof`) reference other concepts by their UCL id.
#[derive(Clone, Debug, Deserialize)]
pub struct ConceptSeed {
    /// The concept's UCL id.
    pub ucl: u64,
    /// Optional canonical UCN root that [`KnowledgeBase::resolve`] maps to this id.
    #[serde(default)]
    pub ucn: Option<String>,
    /// ISO 639-3 language for the UCN/lemmas. Defaults to `eng`.
    #[serde(default = "default_lang")]
    pub lang: String,
    /// `nominal` | `verbal` | `adjectival` | `adverbial`.
    pub category: String,
    #[serde(default, rename = "abstract")]
    pub abstract_: bool,
    #[serde(default)]
    pub gloss: Option<String>,
    /// Hypernyms (icl targets), by UCL id.
    #[serde(default)]
    pub icl: Vec<u64>,
    /// Instance-of links (iof targets), by UCL id.
    #[serde(default)]
    pub iof: Vec<u64>,
    /// Natural-language lemmas indexed for [`KnowledgeBase::candidates`].
    #[serde(default)]
    pub lemmas: Vec<String>,
}

fn default_lang() -> String {
    "eng".to_string()
}

#[derive(Deserialize)]
struct SeedFile {
    #[serde(default)]
    concept: Vec<ConceptSeed>,
}

/// Stored concept: features plus ontology adjacency, keyed in the KB by UCL id.
struct Stored {
    features: ConceptFeatures,
    icl: Vec<u64>,
    iof: Vec<u64>,
}

/// In-memory knowledge base.
#[derive(Default)]
pub struct MemKb {
    concepts: HashMap<u64, Stored>,
    /// UCN root -> (lang, ucl id) candidates, for `resolve`.
    by_ucn: HashMap<SmolStr, Vec<(Lang, u64)>>,
    /// (lemma, lang) -> ucl ids, for `candidates`.
    index: HashMap<(SmolStr, Lang), Vec<u64>>,
}

impl MemKb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed from the TOML fixture format (`[[concept]]` tables).
    pub fn from_toml(s: &str) -> Result<Self, KbError> {
        let file: SeedFile =
            toml::from_str(s).map_err(|e| KbError::Storage(format!("invalid seed TOML: {e}")))?;
        let mut kb = MemKb::new();
        for c in file.concept {
            kb.insert(c)?;
        }
        Ok(kb)
    }

    /// Add one concept. Later inserts with the same UCL id overwrite features
    /// but accumulate index/resolve entries.
    pub fn insert(&mut self, seed: ConceptSeed) -> Result<(), KbError> {
        let lang = Lang::new(&seed.lang).map_err(|e| KbError::Storage(e.to_string()))?;
        let category = parse_category(&seed.category)?;

        if let Some(ucn) = &seed.ucn {
            self.by_ucn
                .entry(SmolStr::new(ucn))
                .or_default()
                .push((lang, seed.ucl));
        }
        for lemma in &seed.lemmas {
            self.index
                .entry((SmolStr::new(lemma), lang))
                .or_default()
                .push(seed.ucl);
        }

        self.concepts.insert(
            seed.ucl,
            Stored {
                features: ConceptFeatures {
                    category,
                    abstract_: seed.abstract_,
                    gloss: seed.gloss,
                },
                icl: seed.icl,
                iof: seed.iof,
            },
        );
        Ok(())
    }

    /// Builder-style insert for fluent construction in tests.
    pub fn with(mut self, seed: ConceptSeed) -> Self {
        self.insert(seed).expect("valid concept seed");
        self
    }

    /// Resolve any identity to a known UCL id.
    fn id_of(&self, u: &Uci) -> Option<u64> {
        match u {
            Uci::Ucl { id, .. } => self.concepts.contains_key(id).then_some(*id),
            Uci::Ucn { lang, root, .. } => {
                let candidates = self.by_ucn.get(root)?;
                match lang {
                    Some(l) => candidates.iter().find(|(cl, _)| cl == l).map(|(_, id)| *id),
                    None => candidates.first().map(|(_, id)| *id),
                }
            }
            Uci::Temporary(_) | Uci::Null => None,
        }
    }
}

impl KnowledgeBase for MemKb {
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError> {
        Ok(self.id_of(ucn).map(Uci::ucl))
    }

    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError> {
        let id = self
            .id_of(concept)
            .ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let stored = &self.concepts[&id];
        let mut rels = Vec::with_capacity(stored.icl.len() + stored.iof.len());
        for &parent in &stored.icl {
            rels.push(concept_relation(RelationTag::Icl, id, parent));
        }
        for &parent in &stored.iof {
            rels.push(concept_relation(RelationTag::Iof, id, parent));
        }
        Ok(rels)
    }

    fn features(&self, concept: &Uci) -> Result<Option<ConceptFeatures>, KbError> {
        Ok(self
            .id_of(concept)
            .map(|id| self.concepts[&id].features.clone()))
    }

    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError> {
        let (Some(start), Some(goal)) = (self.id_of(sub), self.id_of(sup)) else {
            return Ok(false);
        };
        if start == goal {
            return Ok(true);
        }
        // DFS up the icl/iof ontology.
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            let Some(stored) = self.concepts.get(&cur) else {
                continue;
            };
            for &parent in stored.icl.iter().chain(stored.iof.iter()) {
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
        let Some(stored) = self.concepts.get(&src) else {
            return Ok(0);
        };
        let holds = match tag {
            RelationTag::Icl => stored.icl.contains(&tgt),
            RelationTag::Iof => stored.iof.contains(&tgt),
            _ => false,
        };
        Ok(if holds { 255 } else { 0 })
    }

    fn candidates(&self, lemma: &str, lang: Lang) -> Result<Vec<Uci>, KbError> {
        Ok(self
            .index
            .get(&(SmolStr::new(lemma), lang))
            .map(|ids| ids.iter().copied().map(Uci::ucl).collect())
            .unwrap_or_default())
    }
}

fn parse_category(s: &str) -> Result<LexCategory, KbError> {
    match s {
        "nominal" | "n" => Ok(LexCategory::Nominal),
        "verbal" | "v" => Ok(LexCategory::Verbal),
        "adjectival" | "j" => Ok(LexCategory::Adjectival),
        "adverbial" | "a" => Ok(LexCategory::Adverbial),
        other => Err(KbError::Storage(format!("unknown lexical category: {other}"))),
    }
}

/// Build a definitional relation between two concepts identified by UCL id,
/// with the UWs inlined (KB relations are concept-to-concept).
fn concept_relation(tag: RelationTag, source_id: u64, target_id: u64) -> Relation {
    Relation {
        tag,
        scope: None,
        source: NodeRef::Inline(Box::new(Uw::new(Uci::ucl(source_id)))),
        target: NodeRef::Inline(Box::new(Uw::new(Uci::ucl(target_id)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MemKb {
        MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap()
    }

    #[test]
    fn resolves_ucn_to_ucl() {
        let kb = fixture();
        assert_eq!(kb.resolve(&Uci::ucn("cat")).unwrap(), Some(Uci::ucl(102121620)));
        // A known UCL resolves to itself.
        assert_eq!(
            kb.resolve(&Uci::ucl(102121620)).unwrap(),
            Some(Uci::ucl(102121620))
        );
        // Unknown / non-resolvable.
        assert_eq!(kb.resolve(&Uci::ucn("nonsense")).unwrap(), None);
        assert_eq!(kb.resolve(&Uci::Null).unwrap(), None);
    }

    #[test]
    fn features_and_gloss() {
        let kb = fixture();
        let f = kb.features(&Uci::ucn("cat")).unwrap().unwrap();
        assert_eq!(f.category, LexCategory::Nominal);
        assert!(!f.abstract_);
        assert!(f.gloss.as_deref().unwrap().contains("feline"));
        assert_eq!(kb.features(&Uci::ucl(999)).unwrap(), None);
    }

    #[test]
    fn is_a_walks_the_ontology() {
        let kb = fixture();
        // cat -> feline -> carnivore -> mammal -> animal
        assert!(kb.is_a(&Uci::ucn("cat"), &Uci::ucn("animal")).unwrap());
        assert!(kb.is_a(&Uci::ucn("cat"), &Uci::ucn("mammal")).unwrap());
        // reflexive
        assert!(kb.is_a(&Uci::ucl(102121620), &Uci::ucl(102121620)).unwrap());
        // not upward
        assert!(!kb.is_a(&Uci::ucn("animal"), &Uci::ucn("cat")).unwrap());
    }

    #[test]
    fn definition_lists_icl_links() {
        let kb = fixture();
        let def = kb.definition(&Uci::ucn("cat")).unwrap();
        assert!(def.iter().any(|r| r.tag == RelationTag::Icl));
        // Unknown concept errors.
        assert!(matches!(
            kb.definition(&Uci::ucn("nonsense")),
            Err(KbError::NotFound(_))
        ));
    }

    #[test]
    fn relation_certainty_reflects_definition() {
        let kb = fixture();
        // cat icl feline is in the definition.
        assert_eq!(
            kb.relation_certainty(RelationTag::Icl, &Uci::ucn("cat"), &Uci::ucl(102120000))
                .unwrap(),
            255
        );
        // cat agt feline is not.
        assert_eq!(
            kb.relation_certainty(RelationTag::Agt, &Uci::ucn("cat"), &Uci::ucl(102120000))
                .unwrap(),
            0
        );
    }

    #[test]
    fn candidates_by_lemma() {
        let kb = fixture();
        let cands = kb.candidates("cat", Lang::ENG).unwrap();
        assert!(cands.contains(&Uci::ucl(102121620)));
        assert!(kb.candidates("cat", Lang::FRA).unwrap().is_empty());
        assert!(kb.candidates("unknownlemma", Lang::ENG).unwrap().is_empty());
    }

    #[test]
    fn usable_as_trait_object() {
        let kb = fixture();
        let dynkb: &dyn KnowledgeBase = &kb;
        assert!(dynkb.resolve(&Uci::ucn("cat")).unwrap().is_some());
    }

    #[test]
    fn programmatic_builder() {
        let kb = MemKb::new().with(ConceptSeed {
            ucl: 1,
            ucn: Some("thing".into()),
            lang: "eng".into(),
            category: "nominal".into(),
            abstract_: true,
            gloss: None,
            icl: vec![],
            iof: vec![],
            lemmas: vec!["thing".into()],
        });
        assert_eq!(kb.resolve(&Uci::ucn("thing")).unwrap(), Some(Uci::ucl(1)));
    }
}
