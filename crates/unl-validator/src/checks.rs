//! The individual validation checks (spec §1.5 formal properties + structural
//! integrity). Each is a pure function `(&UnlGraph, &dyn KnowledgeBase) ->
//! Vec<Diagnostic>`. The [`crate::Validate`] impl runs them all.

use crate::diagnostic::{DiagCode, Diagnostic, Severity};
use unl_core::{Attr, NodeId, NodeRef, RelationTag, Uci, UnlGraph, Uw};
use unl_kb::{KnowledgeBase, Vocabulary};

/// Every UW in the graph, with its node id where one exists (declared nodes, and
/// inline UWs carrying a `node_id`).
fn uws(g: &UnlGraph) -> Vec<(&Uw, Option<NodeId>)> {
    let mut out = Vec::new();
    for (id, uw) in &g.nodes {
        out.push((uw, Some(id.clone())));
    }
    for r in &g.relations {
        for nr in [&r.source, &r.target] {
            if let NodeRef::Inline(uw) = nr {
                out.push((uw, uw.node_id.clone()));
            }
        }
    }
    out
}

/// The UCI a relation argument refers to, if it resolves to one in this graph.
fn ref_uci<'a>(g: &'a UnlGraph, nr: &'a NodeRef) -> Option<&'a Uci> {
    match nr {
        NodeRef::Inline(uw) => Some(&uw.uci),
        NodeRef::Id(id) => g.nodes.get(id).map(|uw| &uw.uci),
        NodeRef::Scope(_) => None,
    }
}

/// Mutually-exclusive attribute families: at most one member may appear on a node.
fn exclusive_groups() -> [&'static [Attr]; 6] {
    [
        &[Attr::Past, Attr::Present, Attr::Future],
        &[Attr::Singular, Attr::Plural, Attr::Dual, Attr::Trial, Attr::Paucal],
        &[Attr::Affirmative, Attr::Negative],
        &[Attr::Active, Attr::Passive, Attr::Middle],
        &[Attr::Male, Attr::Female, Attr::NeuterGender],
        &[Attr::P1, Attr::P2, Attr::P3],
    ]
}

/// Dangling references and incompatible attributes.
pub fn structural_integrity(g: &UnlGraph, _kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    for r in &g.relations {
        for nr in [&r.source, &r.target] {
            if let NodeRef::Id(id) = nr
                && !g.nodes.contains_key(id)
            {
                out.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagCode::DanglingReference,
                        format!("relation '{}' references undeclared node '{}'", r.tag, id.0),
                    )
                    .at(Some(id.clone())),
                );
            }
        }
    }

    let groups = exclusive_groups();
    for (uw, loc) in uws(g) {
        for group in groups {
            let hits: Vec<&Attr> = uw.attributes.iter().filter(|a| group.contains(a)).collect();
            if hits.len() >= 2 {
                out.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagCode::IncompatibleAttributes,
                        format!("mutually-exclusive attributes on one node: {hits:?}"),
                    )
                    .at(loc.clone()),
                );
            }
        }
    }
    out
}

/// Concepts that do not resolve in the KB (UnknownConcept).
pub fn concept_resolution(g: &UnlGraph, kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (uw, loc) in uws(g) {
        if matches!(uw.uci, Uci::Ucl { .. } | Uci::Ucn { .. })
            && matches!(kb.resolve(&uw.uci), Ok(None))
        {
            out.push(
                Diagnostic::new(
                    Severity::Warning,
                    DiagCode::UnknownConcept,
                    format!("concept does not resolve in the KB: {:?}", uw.uci),
                )
                .at(loc.clone()),
            );
        }
    }
    out
}

/// `icl`/`iof` relations that contradict the KB ontology (RelationTypeViolation).
pub fn relation_legality(g: &UnlGraph, kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for r in &g.relations {
        if !matches!(r.tag, RelationTag::Icl | RelationTag::Iof) {
            continue;
        }
        let (Some(sub), Some(sup)) = (ref_uci(g, &r.source), ref_uci(g, &r.target)) else {
            continue;
        };
        // Only adjudicate when the KB knows both endpoints.
        let known = matches!(kb.resolve(sub), Ok(Some(_))) && matches!(kb.resolve(sup), Ok(Some(_)));
        if known && matches!(kb.is_a(sub, sup), Ok(false)) {
            out.push(Diagnostic::new(
                Severity::Warning,
                DiagCode::RelationTypeViolation,
                format!(
                    "'{}' asserts {:?} is a kind of {:?}, which the KB ontology disproves",
                    r.tag, sub, sup
                ),
            ));
        }
    }
    out
}

/// Unresolved pro-forms (completeness): a `00` UW that should have an antecedent.
pub fn completeness(g: &UnlGraph, _kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (uw, loc) in uws(g) {
        if uw.uci == Uci::Null {
            out.push(
                Diagnostic::new(
                    Severity::Warning,
                    DiagCode::UnsaturatedProForm,
                    "unresolved pro-form (00)",
                )
                .at(loc.clone()),
            );
        }
    }
    out
}

/// Duplicate relation arcs (non-redundancy). Detects literal duplicates.
pub fn non_redundancy(g: &UnlGraph, _kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in &g.relations {
        let key = format!("{r:?}");
        if !seen.insert(key) {
            out.push(Diagnostic::new(
                Severity::Info,
                DiagCode::Redundancy,
                format!("duplicate relation arc: '{}'", r.tag),
            ));
        }
    }
    out
}

/// Vocabulary membership: every concept, relation, and attribute used must be in
/// the agent's vocabulary. Out-of-vocabulary terms are errors — the agent has no
/// word for them, so the message is not-understood.
pub fn vocabulary(graph: &UnlGraph, vocab: &Vocabulary) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    for r in &graph.relations {
        if !vocab.allows_relation(r.tag) {
            out.push(Diagnostic::new(
                Severity::Error,
                DiagCode::OutOfVocabulary,
                format!("relation '{}' is not in the agent's vocabulary", r.tag),
            ));
        }
    }

    for (uw, loc) in uws(graph) {
        if matches!(uw.uci, Uci::Ucl { .. } | Uci::Ucn { .. }) && !vocab.knows(&uw.uci) {
            out.push(
                Diagnostic::new(
                    Severity::Error,
                    DiagCode::OutOfVocabulary,
                    format!("concept not in the agent's vocabulary: {:?}", uw.uci),
                )
                .at(loc.clone()),
            );
        }
        for attr in uw.attributes.iter() {
            if !vocab.allows_attribute(attr) {
                out.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagCode::OutOfVocabulary,
                        format!("attribute '@{}' is not in the agent's vocabulary", attr.as_label()),
                    )
                    .at(loc.clone()),
                );
            }
        }
    }
    out
}

/// Entry-head ambiguity (non-ambiguity). Only applies to graphs with declared
/// nodes (list format): if there is no `@entry` and the head is not uniquely
/// determined by the relation structure, flag it.
pub fn non_ambiguity(g: &UnlGraph, _kb: &dyn KnowledgeBase) -> Vec<Diagnostic> {
    if g.nodes.is_empty() || g.entry.is_some() {
        return Vec::new();
    }
    let mut targets = std::collections::HashSet::new();
    for r in &g.relations {
        if let NodeRef::Id(id) = &r.target {
            targets.insert(id.clone());
        }
    }
    let heads: Vec<&NodeId> = g.nodes.keys().filter(|id| !targets.contains(*id)).collect();
    if heads.len() != 1 {
        return vec![Diagnostic::new(
            Severity::Warning,
            DiagCode::AmbiguousEntry,
            format!("no @entry and {} candidate head nodes", heads.len()),
        )];
    }
    Vec::new()
}
