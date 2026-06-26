//! Normalization (NORMA's job): rewrite a graph to canonical form so that
//! paraphrases collapse together (spec §5). Built as a pluggable pipeline of
//! [`NormRule`]s applied to a fixed point — the "minimum now, expansion-ready"
//! decision (manifest §3): deferred rules are added later with one
//! [`Normalizer::register`] call, no refactor.

use unl_core::{Attr, NodeRef, Uci, UnlGraph, Uw};
use unl_kb::KnowledgeBase;

/// Apply `f` to every UW in the graph (declared nodes and inline UWs).
fn for_each_uw_mut(g: &mut UnlGraph, mut f: impl FnMut(&mut Uw)) {
    for uw in g.nodes.values_mut() {
        f(uw);
    }
    for r in &mut g.relations {
        if let NodeRef::Inline(uw) = &mut r.source {
            f(uw);
        }
        if let NodeRef::Inline(uw) = &mut r.target {
            f(uw);
        }
    }
}

/// A normalization rule: a pure `UnlGraph -> UnlGraph` transform that must be
/// idempotent on its own output.
pub trait NormRule {
    /// Stable identifier, for ordering, logging, and selective enable/disable.
    fn id(&self) -> &'static str;
    /// Apply once.
    fn apply(&self, graph: UnlGraph, kb: &dyn KnowledgeBase) -> UnlGraph;
}

/// Collapse the active/passive diathesis. In UNL the argument structure
/// (`agt`/`obj`) already encodes who-does-what regardless of surface voice, so
/// the `@active`/`@passive` markers are redundant for meaning-equivalence:
/// dropping them makes "Peter killed John" and "John was killed by Peter"
/// converge.
pub struct VoiceCollapse;

impl NormRule for VoiceCollapse {
    fn id(&self) -> &'static str {
        "voice-collapse"
    }
    fn apply(&self, mut graph: UnlGraph, _kb: &dyn KnowledgeBase) -> UnlGraph {
        for_each_uw_mut(&mut graph, |uw| {
            uw.attributes
                .0
                .retain(|a| !matches!(a, Attr::Active | Attr::Passive));
        });
        graph
    }
}

/// Map UWs to their canonical concept via the KB. `kb.resolve` sends a UCN (or
/// any synonym that shares a synset) to its canonical UCL, so "murder" and
/// "kill" converge when the KB unifies them. Idempotent: a canonical UCL
/// resolves to itself.
pub struct SynonymCollapse;

impl NormRule for SynonymCollapse {
    fn id(&self) -> &'static str {
        "synonym-collapse"
    }
    fn apply(&self, mut graph: UnlGraph, kb: &dyn KnowledgeBase) -> UnlGraph {
        for_each_uw_mut(&mut graph, |uw| {
            if let Ok(Some(canonical)) = kb.resolve(&uw.uci) {
                uw.uci = canonical;
            }
        });
        graph
    }
}

/// Resolve `00` pro-forms to their antecedents.
///
/// **Rev 1 stub:** wired into the pipeline but performs no substitution —
/// faithful pro-form resolution needs coreference information the graph does not
/// yet carry. Kept registered so the expansion path is a body change, not a
/// pipeline change.
pub struct ProformResolve;

impl NormRule for ProformResolve {
    fn id(&self) -> &'static str {
        "proform-resolve"
    }
    fn apply(&self, graph: UnlGraph, _kb: &dyn KnowledgeBase) -> UnlGraph {
        let _ = Uci::Null; // documents the target form; no-op in Rev 1
        graph
    }
}

/// Orders and runs a set of [`NormRule`]s to a fixed point.
pub struct Normalizer {
    rules: Vec<Box<dyn NormRule>>,
    max_iterations: usize,
}

impl Normalizer {
    /// The Rev 1 minimum rule set (§3): voice-collapse, synonym-collapse,
    /// proform-resolve. Deferred rules (`redundancy-strip`, `rel-generalize`)
    /// and out-of-scope rhetorical/XUNL are added later via [`Self::register`].
    pub fn rev1() -> Self {
        Normalizer {
            rules: vec![
                Box::new(VoiceCollapse),
                Box::new(SynonymCollapse),
                Box::new(ProformResolve),
            ],
            max_iterations: 16,
        }
    }

    /// Build with an explicit rule set.
    pub fn with_rules(rules: Vec<Box<dyn NormRule>>) -> Self {
        Normalizer {
            rules,
            max_iterations: 16,
        }
    }

    /// Add a rule to the pipeline (the expansion path).
    pub fn register(&mut self, rule: Box<dyn NormRule>) {
        self.rules.push(rule);
    }

    /// The ids of the registered rules, in order.
    pub fn rule_ids(&self) -> Vec<&'static str> {
        self.rules.iter().map(|r| r.id()).collect()
    }

    /// Apply all rules to a fixed point (bounded by `max_iterations`).
    pub fn normalize(&self, mut graph: UnlGraph, kb: &dyn KnowledgeBase) -> UnlGraph {
        for _ in 0..self.max_iterations {
            let before = graph.clone();
            for rule in &self.rules {
                graph = rule.apply(graph, kb);
            }
            if graph == before {
                break;
            }
        }
        graph
    }
}

/// Convenience trait so a graph can normalize itself with a given normalizer.
pub trait Normalize {
    fn normalize(&self, normalizer: &Normalizer, kb: &dyn KnowledgeBase) -> UnlGraph;
}

impl Normalize for UnlGraph {
    fn normalize(&self, normalizer: &Normalizer, kb: &dyn KnowledgeBase) -> UnlGraph {
        normalizer.normalize(self.clone(), kb)
    }
}
