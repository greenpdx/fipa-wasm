//! UCL id-range classification (§4.2).
//!
//! A UCL's numeric id alone tells you which side of the open-core boundary a
//! concept lives on — no KB lookup required. WordNet 3.1 synset offsets are
//! 9 digits (max ~2×10⁹), so imported ids never reach the `5_000_000_000+`
//! reserved blocks: collision is structurally impossible.
//!
//! The classifying methods are inherent on [`Uci`] (which lives in this crate),
//! resolving the manifest's apparent placement of them under `unl-kb` — an
//! inherent impl must live with the type it extends.

use crate::uw::Uci;

/// Which side of the open-core boundary a concept's id falls on. Derived purely
/// from the numeric range.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UclRange {
    /// `0 ..= 4_999_999_999` — WordNet-imported (open seed).
    WordNetSeed,
    /// `5_000_000_000 ..= 5_999_999_999` — curated additions (moat).
    Curated,
    /// `6_000_000_000 ..= 6_999_999_999` — proper nouns (moat).
    ProperNoun,
    /// `9_000_000_000 ..= 9_999_999_999` — temporary / experimental.
    Temporary,
    /// Anything else, currently unused.
    Reserved,
}

impl Uci {
    /// Classify a UCL by its id range. Non-UCL identities return `None`.
    pub fn ucl_range(&self) -> Option<UclRange> {
        match self {
            Uci::Ucl { id, .. } => Some(match *id {
                0..=4_999_999_999 => UclRange::WordNetSeed,
                5_000_000_000..=5_999_999_999 => UclRange::Curated,
                6_000_000_000..=6_999_999_999 => UclRange::ProperNoun,
                9_000_000_000..=9_999_999_999 => UclRange::Temporary,
                _ => UclRange::Reserved,
            }),
            _ => None,
        }
    }

    /// True if this concept belongs to the open seed (vs. the curated layer).
    pub fn is_open_seed(&self) -> bool {
        matches!(self.ucl_range(), Some(UclRange::WordNetSeed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_classify_correctly() {
        assert_eq!(Uci::ucl(102121620).ucl_range(), Some(UclRange::WordNetSeed));
        assert_eq!(Uci::ucl(5_000_000_000).ucl_range(), Some(UclRange::Curated));
        assert_eq!(Uci::ucl(6_500_000_000).ucl_range(), Some(UclRange::ProperNoun));
        assert_eq!(Uci::ucl(9_999_999_999).ucl_range(), Some(UclRange::Temporary));
        assert_eq!(Uci::ucl(7_000_000_000).ucl_range(), Some(UclRange::Reserved));
    }

    #[test]
    fn boundaries_are_inclusive() {
        assert_eq!(Uci::ucl(4_999_999_999).ucl_range(), Some(UclRange::WordNetSeed));
        assert_eq!(Uci::ucl(5_999_999_999).ucl_range(), Some(UclRange::Curated));
    }

    #[test]
    fn wordnet_offsets_never_collide_with_moat() {
        // WordNet 3.1 synset offsets are at most 9 digits.
        let max_wordnet_offset: u64 = 999_999_999;
        assert!(max_wordnet_offset < 5_000_000_000);
        assert_eq!(Uci::ucl(max_wordnet_offset).ucl_range(), Some(UclRange::WordNetSeed));
        assert!(Uci::ucl(max_wordnet_offset).is_open_seed());
    }

    #[test]
    fn non_ucl_has_no_range() {
        assert_eq!(Uci::ucn("cat").ucl_range(), None);
        assert_eq!(Uci::Null.ucl_range(), None);
        assert!(!Uci::ucn("cat").is_open_seed());
    }
}
