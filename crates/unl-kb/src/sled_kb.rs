//! [`SledKb`] — the embedded production knowledge base (manifest §4.3's
//! "RocksKb" role), backed by [`sled`] instead of RocksDB.
//!
//! RocksDB needs libclang to build; the workspace deliberately avoids that
//! (the same reason `fipa-wasm-agents` uses sled). sled is pure-Rust, embedded,
//! and edge-deployable (RPi5 / CRServer), which is what the role calls for.
//!
//! Two trees (sled's column families):
//! - `concepts`: UCL id (8-byte big-endian) → JSON [`ConceptRecord`]
//!   (features + UNL-mapped links).
//! - `index`: lemma (UTF-8) → JSON `Vec<u64>` of candidate UCL ids.
//!
//! Compile the store from the WordNet seed with
//! `cargo run -p xtask -- build-kb`, then `SledKb::open(path)` at runtime.

use crate::{ConceptFeatures, KbError, KnowledgeBase, WordNetKb};
use serde::{Deserialize, Serialize};
use std::path::Path;
use unl_core::{Lang, NodeRef, Relation, RelationTag, Uci, Uw};

#[derive(Serialize, Deserialize)]
struct ConceptRecord {
    features: ConceptFeatures,
    /// Outgoing UNL-mapped links: (relation, target UCL id).
    links: Vec<(RelationTag, u64)>,
}

/// Summary of a compile run.
#[derive(Copy, Clone, Debug)]
pub struct BuildStats {
    pub concepts: u64,
    pub lemmas: u64,
}

/// Embedded knowledge base over sled.
pub struct SledKb {
    db: sled::Db,
    concepts: sled::Tree,
    index: sled::Tree,
}

fn storage(e: impl ToString) -> KbError {
    KbError::Storage(e.to_string())
}

impl SledKb {
    /// Open (creating if absent) the embedded store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, KbError> {
        let db = sled::open(path).map_err(storage)?;
        let concepts = db.open_tree("concepts").map_err(storage)?;
        let index = db.open_tree("index").map_err(storage)?;
        Ok(SledKb { db, concepts, index })
    }

    /// Insert a concept programmatically (used by the WordNet compiler and tests).
    pub fn insert_concept(
        &self,
        id: u64,
        features: ConceptFeatures,
        links: Vec<(RelationTag, u64)>,
    ) -> Result<(), KbError> {
        let record = ConceptRecord { features, links };
        let bytes = serde_json::to_vec(&record).map_err(storage)?;
        self.concepts.insert(id.to_be_bytes(), bytes).map_err(storage)?;
        Ok(())
    }

    /// Index a lemma to its candidate UCL ids (in sense order).
    pub fn insert_lemma(&self, lemma: &str, ucls: &[u64]) -> Result<(), KbError> {
        let bytes = serde_json::to_vec(ucls).map_err(storage)?;
        self.index.insert(lemma.as_bytes(), bytes).map_err(storage)?;
        Ok(())
    }

    /// Flush to disk.
    pub fn flush(&self) -> Result<(), KbError> {
        self.db.flush().map_err(storage)?;
        Ok(())
    }

    /// Compile the embedded store from a WordNet seed.
    pub fn build_from_wordnet(
        wordnet: &WordNetKb,
        path: impl AsRef<Path>,
    ) -> Result<(Self, BuildStats), KbError> {
        let kb = SledKb::open(path)?;
        let mut concepts = 0u64;
        let mut err: Option<KbError> = None;
        wordnet.for_each_concept(|id, features, links| {
            if err.is_some() {
                return;
            }
            if let Err(e) = kb.insert_concept(id, features, links) {
                err = Some(e);
            } else {
                concepts += 1;
            }
        })?;
        if let Some(e) = err {
            return Err(e);
        }
        let mut lemmas = 0u64;
        for (lemma, ucls) in wordnet.index_entries() {
            kb.insert_lemma(lemma, &ucls)?;
            lemmas += 1;
        }
        kb.flush()?;
        Ok((kb, BuildStats { concepts, lemmas }))
    }

    fn record(&self, id: u64) -> Result<Option<ConceptRecord>, KbError> {
        match self.concepts.get(id.to_be_bytes()).map_err(storage)? {
            Some(ivec) => Ok(Some(serde_json::from_slice(&ivec).map_err(storage)?)),
            None => Ok(None),
        }
    }

    fn lemma_ids(&self, lemma: &str) -> Result<Vec<u64>, KbError> {
        match self.index.get(lemma.as_bytes()).map_err(storage)? {
            Some(ivec) => serde_json::from_slice(&ivec).map_err(storage),
            None => Ok(Vec::new()),
        }
    }

    fn id_of(&self, u: &Uci) -> Result<Option<u64>, KbError> {
        match u {
            Uci::Ucl { id, .. } => {
                Ok(self.concepts.contains_key(id.to_be_bytes()).map_err(storage)?.then_some(*id))
            }
            Uci::Ucn { root, .. } => Ok(self.lemma_ids(root)?.first().copied()),
            _ => Ok(None),
        }
    }
}

impl KnowledgeBase for SledKb {
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError> {
        Ok(self.id_of(ucn)?.map(Uci::ucl))
    }

    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError> {
        let id = self.id_of(concept)?.ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let record = self.record(id)?.ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let source = Uci::ucl(id);
        Ok(record
            .links
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
        match self.id_of(concept)? {
            Some(id) => Ok(self.record(id)?.map(|r| r.features)),
            None => Ok(None),
        }
    }

    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError> {
        let (Some(start), Some(goal)) = (self.id_of(sub)?, self.id_of(sup)?) else {
            return Ok(false);
        };
        if start == goal {
            return Ok(true);
        }
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            let Some(record) = self.record(cur)? else { continue };
            for &(tag, target) in &record.links {
                if !matches!(tag, RelationTag::Icl | RelationTag::Iof) {
                    continue;
                }
                if target == goal {
                    return Ok(true);
                }
                if seen.insert(target) {
                    stack.push(target);
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
        let (Some(src), Some(tgt)) = (self.id_of(source)?, self.id_of(target)?) else {
            return Ok(0);
        };
        let Some(record) = self.record(src)? else { return Ok(0) };
        let holds = record.links.iter().any(|&(t, target)| t == tag && target == tgt);
        Ok(if holds { 255 } else { 0 })
    }

    fn candidates(&self, lemma: &str, lang: Lang) -> Result<Vec<Uci>, KbError> {
        if lang != Lang::ENG {
            return Ok(Vec::new());
        }
        Ok(self.lemma_ids(lemma)?.into_iter().map(Uci::ucl).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unl_core::LexCategory;

    fn feat(gloss: &str) -> ConceptFeatures {
        ConceptFeatures {
            category: LexCategory::Nominal,
            abstract_: false,
            gloss: Some(gloss.to_string()),
        }
    }

    /// A small ontology: cat(1) -> feline(2) -> animal(3).
    fn fixture() -> (tempfile::TempDir, SledKb) {
        let dir = tempfile::tempdir().unwrap();
        let kb = SledKb::open(dir.path()).unwrap();
        kb.insert_concept(3, feat("a living organism"), vec![]).unwrap();
        kb.insert_concept(2, feat("a lithe carnivore"), vec![(RelationTag::Icl, 3)]).unwrap();
        kb.insert_concept(1, feat("a small feline pet"), vec![(RelationTag::Icl, 2)]).unwrap();
        kb.insert_lemma("cat", &[1]).unwrap();
        kb.insert_lemma("animal", &[3]).unwrap();
        kb.flush().unwrap();
        (dir, kb)
    }

    #[test]
    fn resolve_features_candidates() {
        let (_d, kb) = fixture();
        assert_eq!(kb.resolve(&Uci::ucn("cat")).unwrap(), Some(Uci::ucl(1)));
        assert_eq!(kb.resolve(&Uci::ucn("nope")).unwrap(), None);
        let f = kb.features(&Uci::ucl(1)).unwrap().unwrap();
        assert_eq!(f.category, LexCategory::Nominal);
        assert!(f.gloss.unwrap().contains("feline"));
        assert_eq!(kb.candidates("cat", Lang::ENG).unwrap(), vec![Uci::ucl(1)]);
        assert!(kb.candidates("cat", Lang::FRA).unwrap().is_empty());
    }

    #[test]
    fn is_a_walks_persisted_ontology() {
        let (_d, kb) = fixture();
        assert!(kb.is_a(&Uci::ucn("cat"), &Uci::ucn("animal")).unwrap());
        assert!(kb.is_a(&Uci::ucl(1), &Uci::ucl(1)).unwrap());
        assert!(!kb.is_a(&Uci::ucn("animal"), &Uci::ucn("cat")).unwrap());
    }

    #[test]
    fn definition_and_certainty() {
        let (_d, kb) = fixture();
        let def = kb.definition(&Uci::ucl(1)).unwrap();
        assert_eq!(def.len(), 1);
        assert_eq!(def[0].tag, RelationTag::Icl);
        assert_eq!(
            kb.relation_certainty(RelationTag::Icl, &Uci::ucl(1), &Uci::ucl(2)).unwrap(),
            255
        );
        assert_eq!(
            kb.relation_certainty(RelationTag::Icl, &Uci::ucl(1), &Uci::ucl(3)).unwrap(),
            0 // transitive, not a direct link
        );
    }

    #[test]
    fn reopen_persists() {
        let dir = tempfile::tempdir().unwrap();
        {
            let kb = SledKb::open(dir.path()).unwrap();
            kb.insert_concept(7, feat("entity"), vec![]).unwrap();
            kb.insert_lemma("thing", &[7]).unwrap();
            kb.flush().unwrap();
        }
        let kb = SledKb::open(dir.path()).unwrap();
        assert_eq!(kb.resolve(&Uci::ucn("thing")).unwrap(), Some(Uci::ucl(7)));
    }

    #[test]
    fn usable_as_trait_object() {
        let (_d, kb) = fixture();
        let dynkb: &dyn KnowledgeBase = &kb;
        assert!(dynkb.resolve(&Uci::ucn("cat")).unwrap().is_some());
    }
}
