//! Universal attributes (spec §3): self-referential annotations on a UW.
//!
//! The attribute set is closed and enumerable. Per the resolved design decision
//! (§2.2) the named variants are spelled out so the compiler can check
//! exhaustiveness; [`Attr::Other`] exists *only* as a forward-compatibility
//! hatch for attributes added to the standard after the 2010 revision — never as
//! a shortcut for omitting a known attribute.
//!
//! The variants below cover the families enumerated in the manifest; remaining
//! members of each family are transcribed here as the full §3.3 list lands.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::HashMap;

/// A single universal attribute (spec §3.3), grouped by semantic family.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Attr {
    // Time (absolute + relative)
    Past, Present, Future, Gnomic,
    Ante, Post, Recent, Remote, Simultaneous, Immediate, Since, Until,
    // Aspect
    Progressive, Perfect, Perfective, Imperfective, Inceptive, Terminative,
    Habitual, Iterative, Causative,
    // Specification / definiteness
    Def, Indef, Each, Own, Same, Certain, Only, Both, Either, Wh,
    // Quantification
    Singular, Plural, Dual, Trial, Quadrual, Paucal, Multal, Total, Universal,
    // Polarity
    Affirmative, Negative, Dubitative, Neutral,
    // Voice
    Active, Passive, Middle, Reflexive, Reciprocal, Anticausative, Impersonal,
    // Person
    P1, P2, P3,
    // Gender / animacy
    Male, Female, NeuterGender, Animal, Person, Thing,
    // Modality
    Ability, Advice, Belief, Command, Request, Desire, Necessity,
    Possibility, Obligation, Permission,
    // Place (location / position / direction)
    Superior, Inferior, Interior, Exterior, Anterior, Posterior,
    Adjacent, Proximal, Distal, Destination, Origin, Transversal,
    // Pragmatics: emotions, register, social deixis, figures of speech
    Anger, Joy, Pain, Surprise,
    Formal, Colloquial, Slang, Jargon, Technical,
    Polite, Familiar, Intimate, Reverential,
    Metaphor, Metonymy, Hyperbole, Irony,
    // Conjunctive shorthand used in corpora (e.g. `.@but`, `.@although`)
    But, Although,
    /// Any attribute not in this enum, preserved verbatim (e.g. `"@foo"`).
    Other(SmolStr),
}

/// An ordered list of attributes attached to a UW. Order is preserved (some
/// pipelines care about it), but equality is **order-insensitive** — two lists
/// are equal iff they are equal as multisets.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AttrList(pub Vec<Attr>);

impl AttrList {
    pub fn new() -> Self {
        AttrList(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn push(&mut self, attr: Attr) {
        self.0.push(attr);
    }

    pub fn contains(&self, attr: &Attr) -> bool {
        self.0.contains(attr)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Attr> {
        self.0.iter()
    }
}

impl FromIterator<Attr> for AttrList {
    fn from_iter<T: IntoIterator<Item = Attr>>(iter: T) -> Self {
        AttrList(iter.into_iter().collect())
    }
}

impl PartialEq for AttrList {
    /// Multiset equality: order is ignored, multiplicity is not.
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        let mut counts: HashMap<&Attr, isize> = HashMap::new();
        for a in &self.0 {
            *counts.entry(a).or_default() += 1;
        }
        for a in &other.0 {
            *counts.entry(a).or_default() -= 1;
        }
        counts.values().all(|&c| c == 0)
    }
}

impl Eq for AttrList {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_is_order_insensitive() {
        let a = AttrList(vec![Attr::Def, Attr::Singular, Attr::Past]);
        let b = AttrList(vec![Attr::Past, Attr::Def, Attr::Singular]);
        assert_eq!(a, b);
    }

    #[test]
    fn equality_respects_multiplicity() {
        let a = AttrList(vec![Attr::Negative, Attr::Negative]);
        let b = AttrList(vec![Attr::Negative]);
        assert_ne!(a, b);
    }

    #[test]
    fn other_hatch_is_distinct() {
        let a = AttrList(vec![Attr::Other("foo".into())]);
        let b = AttrList(vec![Attr::Other("bar".into())]);
        assert_ne!(a, b);
        assert_eq!(a, AttrList(vec![Attr::Other("foo".into())]));
    }

    #[test]
    fn collects_from_iter() {
        let l: AttrList = [Attr::Plural, Attr::Def].into_iter().collect();
        assert_eq!(l.len(), 2);
        assert!(l.contains(&Attr::Plural));
    }
}
