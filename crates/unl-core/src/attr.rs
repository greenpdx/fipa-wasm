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

/// Defines the [`Attr`] enum together with the canonical text label of each
/// variant, single-sourcing the two so they cannot drift. The labels are the
/// snake_case names mirrored in `data/attributes.toml`, and are what the
/// `unl-parser` reads/writes after the `@` sigil (e.g. `Attr::Def` ⇄ `@def`).
macro_rules! define_attrs {
    ($( $variant:ident = $label:literal ),+ $(,)?) => {
        /// A single universal attribute (spec §3.3), grouped by semantic family.
        ///
        /// `Other` is a forward-compatibility hatch only; its payload must be a
        /// non-standard label (one not equal to any named variant's label),
        /// otherwise [`Attr::from_label`] would canonicalize it to the named
        /// variant and the round-trip would not be identity.
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[non_exhaustive]
        pub enum Attr {
            $( $variant, )+
            /// Any attribute not in this enum, preserved verbatim (e.g. `@foo`).
            Other(SmolStr),
        }

        impl Attr {
            /// The canonical text label (without the `@` sigil).
            pub fn as_label(&self) -> &str {
                match self {
                    $( Attr::$variant => $label, )+
                    Attr::Other(s) => s.as_str(),
                }
            }

            /// Parse a label back to an attribute. Unknown labels become
            /// [`Attr::Other`] (the set is open at the bottom for forward compat).
            pub fn from_label(label: &str) -> Attr {
                match label {
                    $( $label => Attr::$variant, )+
                    other => Attr::Other(SmolStr::new(other)),
                }
            }

            /// Every named variant (excludes `Other`), for tests and tooling.
            pub const NAMED: &'static [Attr] = &[ $( Attr::$variant, )+ ];
        }
    };
}

define_attrs! {
    // Time (absolute + relative)
    Past = "past", Present = "present", Future = "future", Gnomic = "gnomic",
    Ante = "ante", Post = "post", Recent = "recent", Remote = "remote",
    Simultaneous = "simultaneous", Immediate = "immediate", Since = "since", Until = "until",
    // Aspect
    Progressive = "progressive", Perfect = "perfect", Perfective = "perfective",
    Imperfective = "imperfective", Inceptive = "inceptive", Terminative = "terminative",
    Habitual = "habitual", Iterative = "iterative", Causative = "causative",
    // Specification / definiteness
    Def = "def", Indef = "indef", Each = "each", Own = "own", Same = "same",
    Certain = "certain", Only = "only", Both = "both", Either = "either", Wh = "wh",
    // Quantification
    Singular = "singular", Plural = "plural", Dual = "dual", Trial = "trial",
    Quadrual = "quadrual", Paucal = "paucal", Multal = "multal", Total = "total",
    Universal = "universal",
    // Polarity
    Affirmative = "affirmative", Negative = "negative", Dubitative = "dubitative",
    Neutral = "neutral",
    // Voice
    Active = "active", Passive = "passive", Middle = "middle", Reflexive = "reflexive",
    Reciprocal = "reciprocal", Anticausative = "anticausative", Impersonal = "impersonal",
    // Person
    P1 = "p1", P2 = "p2", P3 = "p3",
    // Gender / animacy
    Male = "male", Female = "female", NeuterGender = "neuter_gender", Animal = "animal",
    Person = "person", Thing = "thing",
    // Modality
    Ability = "ability", Advice = "advice", Belief = "belief", Command = "command",
    Request = "request", Desire = "desire", Necessity = "necessity",
    Possibility = "possibility", Obligation = "obligation", Permission = "permission",
    // Place (location / position / direction)
    Superior = "superior", Inferior = "inferior", Interior = "interior",
    Exterior = "exterior", Anterior = "anterior", Posterior = "posterior",
    Adjacent = "adjacent", Proximal = "proximal", Distal = "distal",
    Destination = "destination", Origin = "origin", Transversal = "transversal",
    // Pragmatics: emotions, register, social deixis, figures of speech
    Anger = "anger", Joy = "joy", Pain = "pain", Surprise = "surprise",
    Formal = "formal", Colloquial = "colloquial", Slang = "slang", Jargon = "jargon",
    Technical = "technical",
    Polite = "polite", Familiar = "familiar", Intimate = "intimate",
    Reverential = "reverential",
    Metaphor = "metaphor", Metonymy = "metonymy", Hyperbole = "hyperbole", Irony = "irony",
    // Conjunctive shorthand used in corpora (e.g. `@but`, `@although`)
    But = "but", Although = "although",
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

    #[test]
    fn label_roundtrips_for_every_named_variant() {
        for a in Attr::NAMED {
            assert_eq!(Attr::from_label(a.as_label()), a.clone());
        }
    }

    #[test]
    fn named_labels_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for a in Attr::NAMED {
            assert!(seen.insert(a.as_label()), "duplicate label {}", a.as_label());
        }
    }

    #[test]
    fn unknown_label_becomes_other() {
        assert_eq!(Attr::from_label("x_foo"), Attr::Other("x_foo".into()));
        assert_eq!(Attr::Other("x_foo".into()).as_label(), "x_foo");
    }

    #[test]
    fn known_label_matches_attributes_toml() {
        // Spot-check a few labels against data/attributes.toml.
        assert_eq!(Attr::Def.as_label(), "def");
        assert_eq!(Attr::NeuterGender.as_label(), "neuter_gender");
        assert_eq!(Attr::P1.as_label(), "p1");
    }
}
