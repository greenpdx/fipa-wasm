use crate::*;
use unl_core::*;
use unl_kb::MemKb;

fn kb() -> MemKb {
    MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap()
}

fn inline(uci: Uci, attrs: Vec<Attr>) -> NodeRef {
    NodeRef::Inline(Box::new(Uw {
        uci,
        attributes: AttrList(attrs),
        node_id: None,
        scope: None,
    }))
}

fn rel(tag: RelationTag, s: NodeRef, t: NodeRef) -> Relation {
    Relation {
        tag,
        scope: None,
        source: s,
        target: t,
    }
}

fn has_code(diags: &[Diagnostic], code: DiagCode) -> bool {
    diags.iter().any(|d| d.code == code)
}

// --- structural ---------------------------------------------------------

#[test]
fn dangling_reference() {
    let mut g = UnlGraph::new();
    g.insert_node("01", Uw::new(Uci::ucn("kill")));
    g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
    let diags = g.validate(&kb());
    assert!(has_code(&diags, DiagCode::DanglingReference));
    assert!(has_errors(&diags));
}

#[test]
fn incompatible_attributes() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Aoj,
        inline(Uci::ucn("x"), vec![Attr::Past, Attr::Future]),
        inline(Uci::ucn("y"), vec![]),
    ));
    assert!(has_code(&g.validate(&kb()), DiagCode::IncompatibleAttributes));
}

#[test]
fn compatible_attributes_ok() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Aoj,
        inline(Uci::ucn("x"), vec![Attr::Past, Attr::Singular, Attr::Def]),
        inline(Uci::ucn("y"), vec![]),
    ));
    assert!(!has_code(&g.validate(&kb()), DiagCode::IncompatibleAttributes));
}

// --- KB-backed ----------------------------------------------------------

#[test]
fn unknown_concept_flagged_known_concept_clean() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Mod,
        inline(Uci::ucn("cat"), vec![]),       // in the fixture KB
        inline(Uci::ucn("flibbertigibbet"), vec![]), // not
    ));
    let diags = g.validate(&kb());
    assert!(has_code(&diags, DiagCode::UnknownConcept));
    // Exactly one unknown (cat resolves).
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.code == DiagCode::UnknownConcept)
            .count(),
        1
    );
}

#[test]
fn icl_consistent_with_ontology_is_clean() {
    // cat icl animal — true in the KB (cat -> ... -> animal).
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Icl,
        inline(Uci::ucn("cat"), vec![]),
        inline(Uci::ucn("animal"), vec![]),
    ));
    assert!(!has_code(&g.validate(&kb()), DiagCode::RelationTypeViolation));
}

#[test]
fn icl_contradicting_ontology_flagged() {
    // animal icl cat — false (animal is not a kind of cat).
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Icl,
        inline(Uci::ucn("animal"), vec![]),
        inline(Uci::ucn("cat"), vec![]),
    ));
    assert!(has_code(&g.validate(&kb()), DiagCode::RelationTypeViolation));
}

// --- completeness / redundancy / ambiguity ------------------------------

#[test]
fn unresolved_proform() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Agt,
        inline(Uci::ucn("stop"), vec![]),
        inline(Uci::Null, vec![]),
    ));
    assert!(has_code(&g.validate(&kb()), DiagCode::UnsaturatedProForm));
}

#[test]
fn duplicate_relation_is_redundant() {
    let mut g = UnlGraph::new();
    let r = rel(
        RelationTag::And,
        inline(Uci::ucn("a"), vec![]),
        inline(Uci::ucn("b"), vec![]),
    );
    g.add_relation(r.clone());
    g.add_relation(r);
    assert!(has_code(&g.validate(&kb()), DiagCode::Redundancy));
}

#[test]
fn ambiguous_entry() {
    let mut g = UnlGraph::new();
    g.insert_node("01", Uw::new(Uci::ucn("a")));
    g.insert_node("02", Uw::new(Uci::ucn("b")));
    // Two disconnected nodes, no @entry => ambiguous head.
    assert!(has_code(&g.validate(&kb()), DiagCode::AmbiguousEntry));

    g.entry = Some(NodeId::from("01"));
    assert!(!has_code(&g.validate(&kb()), DiagCode::AmbiguousEntry));
}

// --- normalization ------------------------------------------------------

#[test]
fn rev1_rule_order() {
    assert_eq!(
        Normalizer::rev1().rule_ids(),
        vec!["voice-collapse", "synonym-collapse", "proform-resolve"]
    );
}

#[test]
fn voice_collapse_strips_passive() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Obj,
        inline(Uci::ucn("kill"), vec![Attr::Past, Attr::Passive]),
        inline(Uci::ucn("John"), vec![]),
    ));
    let n = Normalizer::rev1().normalize(g.clone(), &kb());
    let NodeRef::Inline(uw) = &n.relations[0].source else {
        panic!()
    };
    assert!(!uw.attributes.contains(&Attr::Passive));
    assert!(uw.attributes.contains(&Attr::Past)); // tense preserved
}

#[test]
fn synonym_collapse_canonicalizes_to_ucl() {
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Mod,
        inline(Uci::ucn("cat"), vec![]),
        inline(Uci::ucn("animal"), vec![]),
    ));
    let n = g.normalize(&Normalizer::rev1(), &kb());
    let NodeRef::Inline(uw) = &n.relations[0].source else {
        panic!()
    };
    assert_eq!(uw.uci, Uci::ucl(102121620)); // cat's canonical UCL
}

#[test]
fn normalize_is_idempotent() {
    let norm = Normalizer::rev1();
    let mut g = UnlGraph::new();
    g.add_relation(rel(
        RelationTag::Obj,
        inline(Uci::ucn("cat"), vec![Attr::Passive]),
        inline(Uci::ucn("animal"), vec![]),
    ));
    let once = norm.normalize(g, &kb());
    let twice = norm.normalize(once.clone(), &kb());
    assert_eq!(once, twice);
}

#[test]
fn active_passive_paraphrases_are_equivalent() {
    // Same propositional content, one marked passive.
    let active = {
        let mut g = UnlGraph::new();
        g.add_relation(rel(
            RelationTag::Agt,
            inline(Uci::ucn("kill"), vec![Attr::Past]),
            inline(Uci::ucn("Peter"), vec![]),
        ));
        g.add_relation(rel(
            RelationTag::Obj,
            inline(Uci::ucn("kill"), vec![Attr::Past]),
            inline(Uci::ucn("John"), vec![]),
        ));
        g
    };
    let passive = {
        let mut g = UnlGraph::new();
        g.add_relation(rel(
            RelationTag::Agt,
            inline(Uci::ucn("kill"), vec![Attr::Past, Attr::Passive]),
            inline(Uci::ucn("Peter"), vec![]),
        ));
        g.add_relation(rel(
            RelationTag::Obj,
            inline(Uci::ucn("kill"), vec![Attr::Past, Attr::Passive]),
            inline(Uci::ucn("John"), vec![]),
        ));
        g
    };
    let norm = Normalizer::rev1();
    assert!(unl_equivalent(&active, &passive, &norm, &kb()));

    // Drop a relation => no longer equivalent.
    let mut shorter = passive.clone();
    shorter.relations.pop();
    assert!(!unl_equivalent(&active, &shorter, &norm, &kb()));
}

// --- corpus smoke -------------------------------------------------------

#[test]
fn validates_aesop_without_panicking() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/corpus/aesop/aesop_en.unl");
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("skip: AESOP corpus not fetched");
        return;
    };
    let doc = unl_parser::parse_legacy_document(&text).unwrap();
    let empty_kb = MemKb::new();
    let mut saw_unknown = false;
    let mut saw_proform = false;
    for s in &doc.sentences {
        let diags = s.graph.validate(&empty_kb);
        // Empty KB => every real concept is unknown.
        saw_unknown |= diags.iter().any(|d| d.code == DiagCode::UnknownConcept);
        saw_proform |= diags.iter().any(|d| d.code == DiagCode::UnsaturatedProForm);
    }
    assert!(saw_unknown, "expected unknown-concept warnings against an empty KB");
    assert!(saw_proform, "AESOP uses 00 pro-forms");
}
