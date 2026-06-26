//! Fixture tests (canonical spec examples) and the round-trip property tests
//! that pin the manifest's `parse(serialize(g)) == g` invariant.

use crate::{parse_legacy_document, parse_sentence, serialize_list, serialize_table};
use unl_core::*;

fn uw(uci: Uci, attrs: Vec<Attr>) -> Uw {
    Uw {
        uci,
        attributes: AttrList(attrs),
        node_id: None,
        scope: None,
    }
}

// ---------------------------------------------------------------------------
// Deterministic fixtures
// ---------------------------------------------------------------------------

/// "Peter killed John" in table form (inline UWs, no node ids).
#[test]
fn table_fixture_exact_and_roundtrip() {
    let mut g = UnlGraph::new();
    g.add_relation(Relation {
        tag: RelationTag::Agt,
        scope: None,
        source: NodeRef::Inline(Box::new(uw(Uci::ucn("kill"), vec![]))),
        target: NodeRef::Inline(Box::new(uw(Uci::ucn("Peter"), vec![]))),
    });
    g.add_relation(Relation {
        tag: RelationTag::Obj,
        scope: None,
        source: NodeRef::Inline(Box::new(uw(Uci::ucn("kill"), vec![]))),
        target: NodeRef::Inline(Box::new(uw(Uci::ucn("John"), vec![]))),
    });

    let text = serialize_table(&g);
    assert_eq!(text, "agt(kill, Peter)\nobj(kill, John)\n");
    assert_eq!(parse_sentence(&text).unwrap(), g);
}

/// The manifest's exact table example: numeric UCLs plus an attribute.
#[test]
fn parses_manifest_table_example() {
    let g = parse_sentence("aoj(300986027, 102121620.@def)").unwrap();
    assert_eq!(g.relations.len(), 1);
    let r = &g.relations[0];
    assert_eq!(r.tag, RelationTag::Aoj);
    match (&r.source, &r.target) {
        (NodeRef::Inline(s), NodeRef::Inline(t)) => {
            assert_eq!(s.uci, Uci::ucl(300986027));
            assert_eq!(t.uci, Uci::ucl(102121620));
            assert_eq!(t.attributes, AttrList(vec![Attr::Def]));
        }
        _ => panic!("expected inline UWs"),
    }
}

/// "Peter killed John" in list form, with an entry head and a tense attribute.
#[test]
fn list_fixture_exact_and_roundtrip() {
    let mut g = UnlGraph::new();
    g.insert_node("01", uw(Uci::ucn("kill"), vec![Attr::Past]));
    g.insert_node("02", uw(Uci::ucn("Peter"), vec![]));
    g.insert_node("03", uw(Uci::ucn("John"), vec![]));
    g.entry = Some(NodeId::from("01"));
    g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
    g.add_relation(Relation::between(RelationTag::Obj, "01".into(), "03".into()));

    let text = serialize_list(&g);
    assert_eq!(
        text,
        "[W]\n01: kill.@past.@entry\n02: Peter\n03: John\n[/W]\n[R]\nagt(01, 02)\nobj(01, 03)\n[/R]\n"
    );
    assert_eq!(parse_sentence(&text).unwrap(), g);
}

#[test]
fn ucn_suffix_and_authority_roundtrip() {
    // cat(icl>feline) and a fully-qualified UCL.
    let mut g = UnlGraph::new();
    g.add_relation(Relation {
        tag: RelationTag::Icl,
        scope: None,
        source: NodeRef::Inline(Box::new(uw(
            Uci::Ucn {
                lang: None,
                root: "cat".into(),
                suffix: Some(UcnSuffix {
                    relation: RelationTag::Icl,
                    word: "feline".into(),
                }),
            },
            vec![],
        ))),
        target: NodeRef::Inline(Box::new(uw(
            Uci::Ucl {
                authority: Some("kb.crmep.com".into()),
                id: 102121620,
            },
            vec![],
        ))),
    });
    let text = serialize_table(&g);
    assert_eq!(text, "icl(cat(icl>feline), ucl://kb.crmep.com/102121620)\n");
    assert_eq!(parse_sentence(&text).unwrap(), g);
}

#[test]
fn null_and_temporary_roundtrip() {
    let mut g = UnlGraph::new();
    g.add_relation(Relation {
        tag: RelationTag::Mod,
        scope: None,
        source: NodeRef::Inline(Box::new(uw(Uci::Null, vec![]))),
        target: NodeRef::Inline(Box::new(uw(
            Uci::Temporary("UNDL Foundation".into()),
            vec![],
        ))),
    });
    let text = serialize_table(&g);
    assert_eq!(text, "mod(00, \"UNDL Foundation\")\n");
    assert_eq!(parse_sentence(&text).unwrap(), g);
}

#[test]
fn deferred_document_formats_report_unsupported() {
    assert!(matches!(
        parse_legacy_document("[D]...[/D]"),
        Err(crate::ParseError::Unsupported(_))
    ));
}

#[test]
fn syntax_error_reports_offset() {
    // Unknown relation tag.
    let err = parse_sentence("zzz(a, b)").unwrap_err();
    assert!(matches!(err, crate::ParseError::Core(_)));
    // Missing close paren.
    let err = parse_sentence("agt(a, b").unwrap_err();
    assert!(matches!(err, crate::ParseError::Syntax { .. }));
}

// ---------------------------------------------------------------------------
// Property-based round-trip
// ---------------------------------------------------------------------------

mod roundtrip {
    use super::*;
    use proptest::prelude::*;

    fn arb_attr() -> impl Strategy<Value = Attr> {
        let named: Vec<Attr> = Attr::NAMED.to_vec();
        prop_oneof![
            9 => proptest::sample::select(named),
            // `Other` payloads are namespaced so they never collide with a
            // known label (which would canonicalize on parse).
            1 => "[a-z]{1,5}".prop_map(|s| Attr::Other(format!("x_{s}").into())),
        ]
    }

    fn arb_temporary() -> impl Strategy<Value = String> {
        // Includes the escape-sensitive characters and a multibyte char.
        proptest::collection::vec(
            proptest::sample::select(vec!['a', 'B', '7', ' ', 'x', '"', '\\', 'é']),
            1..8,
        )
        .prop_map(|cs| cs.into_iter().collect())
    }

    fn arb_uci() -> impl Strategy<Value = Uci> {
        let reltags: Vec<RelationTag> = RelationTag::ALL.to_vec();
        prop_oneof![
            1 => Just(Uci::Null),
            3 => (0u64..=9_999_999_999u64).prop_map(|id| Uci::Ucl { authority: None, id }),
            1 => (0u64..=9_999_999_999u64)
                .prop_map(|id| Uci::Ucl { authority: Some("kb.crmep.com".into()), id }),
            3 => (
                    "[a-z][a-z0-9]{0,7}",
                    proptest::option::of(("[a-z][a-z0-9]{0,7}", proptest::sample::select(reltags))),
                 )
                 .prop_map(|(root, suf)| Uci::Ucn {
                     lang: None,
                     root: root.into(),
                     suffix: suf.map(|(word, relation)| UcnSuffix { relation, word: word.into() }),
                 }),
            2 => arb_temporary().prop_map(|s| Uci::Temporary(s.into())),
        ]
    }

    fn arb_uw() -> impl Strategy<Value = Uw> {
        (arb_uci(), proptest::collection::vec(arb_attr(), 0..4))
            .prop_map(|(uci, attrs)| super::uw(uci, attrs))
    }

    fn arb_opt_scope() -> impl Strategy<Value = Option<ScopeId>> {
        proptest::option::of("[a-z0-9]{1,3}".prop_map(|s| ScopeId(s.into())))
    }

    fn arb_tag() -> impl Strategy<Value = RelationTag> {
        proptest::sample::select(RelationTag::ALL.to_vec())
    }

    prop_compose! {
        fn arb_table_graph()(
            rels in proptest::collection::vec(
                (arb_tag(), arb_opt_scope(), arb_uw(), arb_uw()), 0..6)
        ) -> UnlGraph {
            let mut g = UnlGraph::new();
            for (tag, scope, s, t) in rels {
                g.add_relation(Relation {
                    tag, scope,
                    source: NodeRef::Inline(Box::new(s)),
                    target: NodeRef::Inline(Box::new(t)),
                });
            }
            g
        }
    }

    fn arb_list_graph() -> impl Strategy<Value = UnlGraph> {
        proptest::collection::vec(arb_uw(), 1..5).prop_flat_map(|uws| {
            let n = uws.len();
            let ids: Vec<String> = (0..n).map(|i| format!("n{i}")).collect();
            let rels = proptest::collection::vec(
                (arb_tag(), arb_opt_scope(), 0..n, 0..n),
                0..6,
            );
            let entry = proptest::option::of(0..n);
            (Just(uws), Just(ids), rels, entry).prop_map(|(uws, ids, rels, entry)| {
                let mut g = UnlGraph::new();
                for (id, uw) in ids.iter().zip(uws) {
                    g.insert_node(id.clone(), uw);
                }
                for (tag, scope, si, ti) in rels {
                    g.add_relation(Relation {
                        tag,
                        scope,
                        source: NodeRef::Id(NodeId(ids[si].clone().into())),
                        target: NodeRef::Id(NodeId(ids[ti].clone().into())),
                    });
                }
                g.entry = entry.map(|i| NodeId(ids[i].clone().into()));
                g
            })
        })
    }

    proptest! {
        #[test]
        fn table_roundtrips(g in arb_table_graph()) {
            let text = serialize_table(&g);
            let parsed = parse_sentence(&text)
                .map_err(|e| TestCaseError::fail(format!("parse error: {e}\n--- text ---\n{text}")))?;
            prop_assert_eq!(parsed, g);
        }

        #[test]
        fn list_roundtrips(g in arb_list_graph()) {
            let text = serialize_list(&g);
            let parsed = parse_sentence(&text)
                .map_err(|e| TestCaseError::fail(format!("parse error: {e}\n--- text ---\n{text}")))?;
            prop_assert_eq!(parsed, g);
        }

        /// Serialization is deterministic: same graph, same bytes.
        #[test]
        fn table_serialization_is_stable(g in arb_table_graph()) {
            prop_assert_eq!(serialize_table(&g), serialize_table(&g));
        }
    }
}
