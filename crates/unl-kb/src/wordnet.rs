//! [`WordNetKb`] — the open seed knowledge base, reading the Princeton WordNet
//! 3.1 database files directly (manifest §4.3). Fetch the data with
//! `cargo run -p xtask -- fetch-wordnet`.
//!
//! ## WordNet → UNL mapping (§4.3)
//!
//! | WordNet pointer        | symbol         | UNL relation |
//! |------------------------|----------------|--------------|
//! | hypernym               | `@`            | `icl`        |
//! | instance hypernym      | `@i`           | `iof`        |
//! | part/member/substance holonym | `#p`/`#m`/`#s` | `pof` |
//! | antonym                | `!`            | `ant`        |
//! | similar-to             | `&`            | `equ` (weak) |
//! | domain (topic/region/usage) | `;c`/`;r`/`;u` | `fld`   |
//!
//! Other pointers (hyponyms, meronyms, derivations, …) are ignored in Rev 1.
//! A synset's gloss becomes [`ConceptFeatures::gloss`]; its POS becomes the
//! [`LexCategory`].
//!
//! ## UCL id scheme — deviation from §4.2, flagged
//!
//! The manifest says "id = WordNet synset offset, verbatim". That cannot be
//! literally correct: WordNet offsets are *byte positions within a per-POS data
//! file*, so the same number denotes different synsets across parts of speech
//! (e.g. `1740` is the noun *entity* and the verb *breathe* — 95 such collisions
//! between nouns and verbs alone). A bare offset is therefore not a unique UCL.
//!
//! Rev 1 resolves this by placing each POS in its own billion-block, all inside
//! the `0..5_000_000_000` open-seed range (§4.2):
//!
//! | POS  | block base       |
//! |------|------------------|
//! | noun | `0`              |
//! | verb | `1_000_000_000`  |
//! | adj  | `2_000_000_000`  |
//! | adv  | `3_000_000_000`  |
//!
//! The raw offset stays recoverable as `id % 1_000_000_000` and the POS as
//! `id / 1_000_000_000`, so the manifest's interop hypothesis (do corpus UCLs
//! equal WordNet offsets?) is still testable — that is the open **M2 verification
//! gate**, which this collision finding directly informs.

use crate::{ConceptFeatures, KbError, KnowledgeBase};
use smol_str::SmolStr;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use unl_core::{Lang, LexCategory, NodeRef, Relation, RelationTag, Uci, Uw};

const BLOCK: u64 = 1_000_000_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

impl Pos {
    const ALL: [Pos; 4] = [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv];

    /// Maps both `a` (adjective) and `s` (adjective satellite) to `Adj`.
    fn from_char(c: char) -> Option<Pos> {
        match c {
            'n' => Some(Pos::Noun),
            'v' => Some(Pos::Verb),
            'a' | 's' => Some(Pos::Adj),
            'r' => Some(Pos::Adv),
            _ => None,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Verb => "verb",
            Pos::Adj => "adj",
            Pos::Adv => "adv",
        }
    }

    fn lex_category(self) -> LexCategory {
        match self {
            Pos::Noun => LexCategory::Nominal,
            Pos::Verb => LexCategory::Verbal,
            Pos::Adj => LexCategory::Adjectival,
            Pos::Adv => LexCategory::Adverbial,
        }
    }

    fn block(self) -> u64 {
        match self {
            Pos::Noun => 0,
            Pos::Verb => BLOCK,
            Pos::Adj => 2 * BLOCK,
            Pos::Adv => 3 * BLOCK,
        }
    }

    fn ucl(self, offset: u32) -> u64 {
        self.block() + offset as u64
    }

    fn from_ucl(id: u64) -> Option<(Pos, u32)> {
        let pos = match id / BLOCK {
            0 => Pos::Noun,
            1 => Pos::Verb,
            2 => Pos::Adj,
            3 => Pos::Adv,
            _ => return None,
        };
        Some((pos, (id % BLOCK) as u32))
    }
}

/// Translate a WordNet pointer symbol to a UNL relation, per §4.3. Returns
/// `None` for pointers Rev 1 does not map.
fn map_pointer(symbol: &str) -> Option<RelationTag> {
    Some(match symbol {
        "@" => RelationTag::Icl,
        "@i" => RelationTag::Iof,
        "#p" | "#m" | "#s" => RelationTag::Pof,
        "!" => RelationTag::Ant,
        "&" => RelationTag::Equ,
        ";c" | ";r" | ";u" => RelationTag::Fld,
        _ => return None,
    })
}

/// One parsed synset: its gloss and the UNL-mapped outgoing links.
struct Synset {
    gloss: String,
    /// (relation, target POS, target offset) for mapped pointers only.
    links: Vec<(RelationTag, Pos, u32)>,
}

/// WordNet-backed knowledge base. The lemma index is loaded into memory at
/// construction; synsets are read lazily by seeking to their byte offset in the
/// per-POS data file.
pub struct WordNetKb {
    dict: PathBuf,
    /// lemma -> ordered (POS, offset) senses, across all parts of speech.
    index: HashMap<SmolStr, Vec<(Pos, u32)>>,
}

impl WordNetKb {
    /// Open a WordNet `dict/` directory (the one produced by `xtask
    /// fetch-wordnet`), loading the lemma indexes.
    pub fn open(dict_dir: impl AsRef<Path>) -> Result<Self, KbError> {
        let dict = dict_dir.as_ref().to_path_buf();
        if !dict.join("data.noun").exists() {
            return Err(KbError::Storage(format!(
                "no data.noun under {} — run `cargo run -p xtask -- fetch-wordnet`",
                dict.display()
            )));
        }
        let mut index: HashMap<SmolStr, Vec<(Pos, u32)>> = HashMap::new();
        for pos in Pos::ALL {
            load_index(&dict.join(format!("index.{}", pos.suffix())), pos, &mut index)?;
        }
        Ok(WordNetKb { dict, index })
    }

    /// Resolve any identity to a concrete (POS, offset) synset address.
    fn locate(&self, u: &Uci) -> Option<(Pos, u32)> {
        match u {
            Uci::Ucl { id, .. } => Pos::from_ucl(*id),
            Uci::Ucn { root, .. } => self.index.get(root).and_then(|v| v.first()).copied(),
            _ => None,
        }
    }

    /// Read and parse one synset by seeking to its byte offset.
    fn read_synset(&self, pos: Pos, offset: u32) -> Result<Option<Synset>, KbError> {
        let path = self.dict.join(format!("data.{}", pos.suffix()));
        let mut file = File::open(&path).map_err(|e| KbError::Storage(e.to_string()))?;
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|e| KbError::Storage(e.to_string()))?;
        let mut line = String::new();
        let n = BufReader::new(file)
            .read_line(&mut line)
            .map_err(|e| KbError::Storage(e.to_string()))?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(parse_synset(&line)?))
    }

    fn synset_of(&self, concept: &Uci) -> Result<Option<(Pos, u32, Synset)>, KbError> {
        match self.locate(concept) {
            Some((pos, off)) => Ok(self.read_synset(pos, off)?.map(|s| (pos, off, s))),
            None => Ok(None),
        }
    }
}

impl KnowledgeBase for WordNetKb {
    fn resolve(&self, ucn: &Uci) -> Result<Option<Uci>, KbError> {
        Ok(self.locate(ucn).map(|(pos, off)| Uci::ucl(pos.ucl(off))))
    }

    fn definition(&self, concept: &Uci) -> Result<Vec<Relation>, KbError> {
        let (pos, off, syn) = self
            .synset_of(concept)?
            .ok_or_else(|| KbError::NotFound(concept.clone()))?;
        let source = Uci::ucl(pos.ucl(off));
        Ok(syn
            .links
            .iter()
            .map(|&(tag, tpos, toff)| concept_relation(tag, &source, Uci::ucl(tpos.ucl(toff))))
            .collect())
    }

    fn features(&self, concept: &Uci) -> Result<Option<ConceptFeatures>, KbError> {
        Ok(self.synset_of(concept)?.map(|(pos, _off, syn)| ConceptFeatures {
            category: pos.lex_category(),
            // WordNet does not flag abstractness directly; left false in Rev 1.
            abstract_: false,
            gloss: Some(syn.gloss),
        }))
    }

    fn is_a(&self, sub: &Uci, sup: &Uci) -> Result<bool, KbError> {
        let (Some(start), Some(goal)) = (self.locate(sub), self.locate(sup)) else {
            return Ok(false);
        };
        if start == goal {
            return Ok(true);
        }
        // Walk the hypernym / instance-hypernym DAG upward.
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![start];
        while let Some((pos, off)) = stack.pop() {
            let Some(syn) = self.read_synset(pos, off)? else {
                continue;
            };
            for &(tag, tpos, toff) in &syn.links {
                if !matches!(tag, RelationTag::Icl | RelationTag::Iof) {
                    continue;
                }
                if (tpos, toff) == goal {
                    return Ok(true);
                }
                if seen.insert((tpos, toff)) {
                    stack.push((tpos, toff));
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
        let (Some((pos, off)), Some(goal)) = (self.locate(source), self.locate(target)) else {
            return Ok(0);
        };
        let Some(syn) = self.read_synset(pos, off)? else {
            return Ok(0);
        };
        let holds = syn
            .links
            .iter()
            .any(|&(t, tpos, toff)| t == tag && (tpos, toff) == goal);
        Ok(if holds { 255 } else { 0 })
    }

    fn candidates(&self, lemma: &str, lang: Lang) -> Result<Vec<Uci>, KbError> {
        if lang != Lang::ENG {
            return Ok(Vec::new());
        }
        Ok(self
            .index
            .get(lemma)
            .map(|senses| {
                senses
                    .iter()
                    .map(|&(pos, off)| Uci::ucl(pos.ucl(off)))
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// Parse one `data.*` synset line. Format (before the ` | gloss`):
/// `offset lex_file ss_type w_cnt [word lex_id]... p_cnt [sym off pos st]... [frames]`
fn parse_synset(line: &str) -> Result<Synset, KbError> {
    let (pre, gloss) = line.split_once(" | ").unwrap_or((line, ""));
    let t: Vec<&str> = pre.split_whitespace().collect();
    let bad = |what: &str| KbError::Storage(format!("malformed synset ({what}): {pre}"));

    if t.len() < 4 {
        return Err(bad("too short"));
    }
    let w_cnt = usize::from_str_radix(t[3], 16).map_err(|_| bad("w_cnt"))?;
    let mut i = 4 + 2 * w_cnt; // skip word/lex_id pairs
    let p_cnt: usize = t.get(i).ok_or_else(|| bad("p_cnt"))?.parse().map_err(|_| bad("p_cnt"))?;
    i += 1;

    let mut links = Vec::new();
    for _ in 0..p_cnt {
        let sym = *t.get(i).ok_or_else(|| bad("ptr symbol"))?;
        let toff: u32 = t.get(i + 1).ok_or_else(|| bad("ptr offset"))?.parse().map_err(|_| bad("ptr offset"))?;
        let pchar = t.get(i + 2).and_then(|s| s.chars().next()).ok_or_else(|| bad("ptr pos"))?;
        i += 4;
        if let (Some(tag), Some(tpos)) = (map_pointer(sym), Pos::from_char(pchar)) {
            links.push((tag, tpos, toff));
        }
    }

    Ok(Synset {
        gloss: gloss.trim().to_string(),
        links,
    })
}

fn load_index(
    path: &Path,
    pos: Pos,
    out: &mut HashMap<SmolStr, Vec<(Pos, u32)>>,
) -> Result<(), KbError> {
    let content = std::fs::read_to_string(path).map_err(|e| KbError::Storage(e.to_string()))?;
    for line in content.lines() {
        // Licence header lines begin with two spaces.
        if line.starts_with("  ") || line.is_empty() {
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        // lemma pos synset_cnt p_cnt [sym]... sense_cnt tagsense_cnt off...
        let Some(synset_cnt) = t.get(2).and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        if synset_cnt == 0 || t.len() < synset_cnt {
            continue;
        }
        let lemma = t[0];
        let offsets = &t[t.len() - synset_cnt..];
        let entry = out.entry(SmolStr::new(lemma)).or_default();
        for o in offsets {
            if let Ok(off) = o.parse::<u32>() {
                entry.push((pos, off));
            }
        }
    }
    Ok(())
}

fn concept_relation(tag: RelationTag, source: &Uci, target: Uci) -> Relation {
    Relation {
        tag,
        scope: None,
        source: NodeRef::Inline(Box::new(Uw::new(source.clone()))),
        target: NodeRef::Inline(Box::new(Uw::new(target))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/kb-seed/wordnet-3.1/dict")
    }

    /// Open the real WordNet, or return `None` (skip) if it has not been
    /// downloaded — the data is gitignored, fetched via xtask.
    fn kb() -> Option<WordNetKb> {
        let d = dict_dir();
        if !d.join("data.noun").exists() {
            eprintln!("skipping WordNet test: {} not present (run xtask fetch-wordnet)", d.display());
            return None;
        }
        Some(WordNetKb::open(&d).unwrap())
    }

    // cat (noun, sense 1) and entity (noun root) offsets in WordNet 3.1.
    const CAT_OFFSET: u32 = 2124272;
    const ENTITY_OFFSET: u32 = 1740;

    #[test]
    fn resolve_and_candidates() {
        let Some(kb) = kb() else { return };
        let cat = kb.resolve(&Uci::ucn("cat")).unwrap().unwrap();
        assert_eq!(cat, Uci::ucl(Pos::Noun.ucl(CAT_OFFSET)));
        let cands = kb.candidates("cat", Lang::ENG).unwrap();
        assert!(cands.contains(&cat));
        assert!(cands.len() > 1, "cat has multiple senses");
        assert!(kb.candidates("cat", Lang::FRA).unwrap().is_empty());
    }

    #[test]
    fn features_carry_pos_and_gloss() {
        let Some(kb) = kb() else { return };
        let f = kb
            .features(&Uci::ucl(Pos::Noun.ucl(CAT_OFFSET)))
            .unwrap()
            .unwrap();
        assert_eq!(f.category, LexCategory::Nominal);
        assert!(f.gloss.is_some_and(|g| !g.is_empty()));
    }

    #[test]
    fn cross_pos_offset_collision_is_resolved() {
        let Some(kb) = kb() else { return };
        // Offset 1740 is the noun "entity" AND the verb "breathe". The
        // POS-blocked ids keep them distinct.
        let noun = kb.features(&Uci::ucl(Pos::Noun.ucl(ENTITY_OFFSET))).unwrap().unwrap();
        let verb = kb.features(&Uci::ucl(Pos::Verb.ucl(ENTITY_OFFSET))).unwrap().unwrap();
        assert_eq!(noun.category, LexCategory::Nominal);
        assert_eq!(verb.category, LexCategory::Verbal);
        assert_ne!(Pos::Noun.ucl(ENTITY_OFFSET), Pos::Verb.ucl(ENTITY_OFFSET));
    }

    #[test]
    fn is_a_walks_hypernyms() {
        let Some(kb) = kb() else { return };
        let cat = Uci::ucl(Pos::Noun.ucl(CAT_OFFSET));
        let entity = Uci::ucl(Pos::Noun.ucl(ENTITY_OFFSET));
        assert!(kb.is_a(&cat, &entity).unwrap(), "cat is-a entity");
        assert!(kb.is_a(&cat, &cat).unwrap(), "reflexive");
        assert!(!kb.is_a(&entity, &cat).unwrap(), "not downward");
    }

    #[test]
    fn definition_maps_hypernym_to_icl() {
        let Some(kb) = kb() else { return };
        let def = kb.definition(&Uci::ucl(Pos::Noun.ucl(CAT_OFFSET))).unwrap();
        assert!(
            def.iter().any(|r| r.tag == RelationTag::Icl),
            "cat should have an icl (hypernym) link"
        );
    }

    #[test]
    fn usable_as_trait_object() {
        let Some(kb) = kb() else { return };
        let dynkb: &dyn KnowledgeBase = &kb;
        assert!(dynkb.resolve(&Uci::ucn("cat")).unwrap().is_some());
    }
}
