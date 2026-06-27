//! [`LlmUnlizer`] — the validate-and-repair pipeline (manifest §6).

use crate::{LlmError, Prompt, ReasoningBackend, SemanticGrounder, Unlization, Unlizer};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use unl_core::{
    Attr, AttrList, Lang, NodeId, Relation, RelationTag, Uci, UnlGraph, Uw,
};
use unl_kb::KnowledgeBase;
use unl_validator::{Diagnostic, Severity, Validate};

/// LLM-assisted UNLizer over a reasoning backend `B` and a knowledge base `K`.
pub struct LlmUnlizer<B, K> {
    backend: B,
    kb: K,
    max_repairs: usize,
    grounder: Option<Box<dyn SemanticGrounder>>,
}

impl<B: ReasoningBackend, K: KnowledgeBase> LlmUnlizer<B, K> {
    /// Build with a default of 2 repair attempts and exact-only grounding.
    pub fn new(backend: B, kb: K) -> Self {
        LlmUnlizer { backend, kb, max_repairs: 2, grounder: None }
    }

    /// Set the maximum number of repair retries fed back to the model.
    pub fn with_max_repairs(mut self, n: usize) -> Self {
        self.max_repairs = n;
        self
    }

    /// Augment exact KB grounding with embedding-based semantic retrieval
    /// (the vector-index lever — see [`crate::VectorGrounder`]).
    pub fn with_grounder(mut self, grounder: Box<dyn SemanticGrounder>) -> Self {
        self.grounder = Some(grounder);
        self
    }

    /// Grounding: candidate UCLs for each whitespace token, as hints, plus —
    /// when a [`SemanticGrounder`] is configured — concepts retrieved by meaning
    /// for the whole sentence. (Real lemmatization is deferred; tokenization
    /// suffices for Rev 1.)
    async fn grounding(&self, text: &str, lang: Lang) -> String {
        let mut lines = Vec::new();
        for token in text.split_whitespace() {
            let lemma = token.trim_matches(|c: char| !c.is_alphanumeric());
            if lemma.is_empty() {
                continue;
            }
            if let Ok(cands) = self.kb.candidates(lemma, lang)
                && !cands.is_empty()
            {
                let ids: Vec<String> = cands
                    .iter()
                    .filter_map(|u| match u {
                        Uci::Ucl { id, .. } => Some(id.to_string()),
                        _ => None,
                    })
                    .collect();
                if !ids.is_empty() {
                    lines.push(format!("  {lemma}: {}", ids.join(", ")));
                }
            }
        }
        if let Some(grounder) = &self.grounder
            && let Ok(concepts) = grounder.related_concepts(text, 5).await
        {
            let ids: Vec<String> = concepts
                .iter()
                .filter_map(|u| match u {
                    Uci::Ucl { id, .. } => Some(id.to_string()),
                    _ => None,
                })
                .collect();
            if !ids.is_empty() {
                lines.push(format!("  (semantically related: {})", ids.join(", ")));
            }
        }

        if lines.is_empty() {
            "  (no KB candidates found; use readable headwords as UWs)".to_string()
        } else {
            lines.join("\n")
        }
    }

    async fn base_prompt(&self, text: &str, lang: Lang) -> Prompt {
        let rels: Vec<&str> = RelationTag::ALL.iter().map(|t| t.as_str()).collect();
        let system = format!(
            "You convert a natural-language sentence into a UNL (Universal Networking \
             Language) semantic graph as JSON.\n\
             - A graph has `nodes` (Universal Words) and `relations` (directed binary arcs).\n\
             - Each node has an `id` (you choose, e.g. \"01\"), a `uw` (a KB numeric id when \
             one is given, otherwise the readable headword/lemma), and optional `attrs`.\n\
             - Each relation has `rel` (one of the {n} universal relation tags), `from` and \
             `to` (node ids).\n\
             - Mark the head/main node with `entry`.\n\
             - Use ONLY relation tags from this set: {rels}.\n\
             Return ONLY the JSON object, nothing else.",
            n = rels.len(),
            rels = rels.join(", ")
        );
        let user = format!(
            "Language: {lang}\nSentence: {text}\n\nKB candidate concepts (lemma: ids):\n{hints}\n\n\
             Produce the UNL graph JSON.",
            hints = self.grounding(text, lang).await
        );
        Prompt {
            system,
            user,
            format: Some(graph_schema()),
        }
    }
}

#[async_trait]
impl<B: ReasoningBackend, K: KnowledgeBase + Send + Sync> Unlizer for LlmUnlizer<B, K> {
    async fn unlize(&self, text: &str, lang: Lang) -> Result<Unlization, LlmError> {
        let base = self.base_prompt(text, lang).await;
        let mut prompt = base.clone();
        let mut last_error: Option<String> = None;

        for attempt in 0..=self.max_repairs {
            let raw = self.backend.complete(&prompt).await?;
            match decode_graph(&raw) {
                Ok(graph) => {
                    let diagnostics = graph.validate(&self.kb);
                    let errors = diagnostics
                        .iter()
                        .filter(|d| d.severity == Severity::Error)
                        .count();
                    if errors == 0 || attempt == self.max_repairs {
                        let confidence = self.confidence(&graph, &diagnostics);
                        return Ok(Unlization {
                            graph,
                            residual_diagnostics: diagnostics,
                            confidence,
                        });
                    }
                    prompt = repair_prompt(&base, &raw, &diagnostics);
                }
                Err(msg) => {
                    last_error = Some(msg.clone());
                    if attempt == self.max_repairs {
                        return Err(LlmError::Decode(msg));
                    }
                    prompt = parse_repair_prompt(&base, &raw, &msg);
                }
            }
        }
        Err(LlmError::Decode(
            last_error.unwrap_or_else(|| "exhausted repair attempts".to_string()),
        ))
    }
}

impl<B: ReasoningBackend, K: KnowledgeBase> LlmUnlizer<B, K> {
    /// Heuristic aggregate confidence: fraction of nodes whose concept resolves
    /// in the KB, minus a small penalty per residual diagnostic.
    fn confidence(&self, graph: &UnlGraph, diagnostics: &[Diagnostic]) -> f32 {
        let total = graph.nodes.len().max(1) as f32;
        let resolved = graph
            .nodes
            .values()
            .filter(|uw| matches!(self.kb.resolve(&uw.uci), Ok(Some(_))))
            .count() as f32;
        let penalty = 0.05 * diagnostics.len() as f32;
        (resolved / total - penalty).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// JSON schema + decode
// ---------------------------------------------------------------------------

/// The constrained output schema: relation/attribute fields are `enum`s over the
/// closed sets, so the model selects a tag rather than inventing one.
fn graph_schema() -> serde_json::Value {
    let rels: Vec<&str> = RelationTag::ALL.iter().map(|t| t.as_str()).collect();
    let attrs: Vec<&str> = Attr::NAMED.iter().map(|a| a.as_label()).collect();
    json!({
        "type": "object",
        "properties": {
            "nodes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "uw": { "type": "string" },
                        "attrs": { "type": "array", "items": { "type": "string", "enum": attrs } }
                    },
                    "required": ["id", "uw"]
                }
            },
            "relations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "rel": { "type": "string", "enum": rels },
                        "from": { "type": "string" },
                        "to": { "type": "string" }
                    },
                    "required": ["rel", "from", "to"]
                }
            },
            "entry": { "type": "string" }
        },
        "required": ["nodes", "relations"]
    })
}

#[derive(Deserialize)]
struct GraphDto {
    #[serde(default)]
    nodes: Vec<NodeDto>,
    #[serde(default)]
    relations: Vec<RelDto>,
    #[serde(default)]
    entry: Option<String>,
}

#[derive(Deserialize)]
struct NodeDto {
    id: String,
    uw: String,
    #[serde(default)]
    attrs: Vec<String>,
}

#[derive(Deserialize)]
struct RelDto {
    rel: String,
    from: String,
    to: String,
}

/// Parse the model's JSON and build a `UnlGraph`, validating tags against the
/// closed relation set. Returns a human-readable message on failure (fed back
/// to the model for repair).
fn decode_graph(raw: &str) -> Result<UnlGraph, String> {
    let json = extract_json(raw);
    let dto: GraphDto =
        serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;

    let mut graph = UnlGraph::new();
    for n in dto.nodes {
        let uci = parse_uw(&n.uw);
        let attributes: AttrList = n.attrs.iter().map(|a| Attr::from_label(a)).collect();
        graph.nodes.insert(
            NodeId::from(n.id),
            Uw { uci, attributes, node_id: None, scope: None },
        );
    }
    for r in dto.relations {
        let tag = r
            .rel
            .parse::<RelationTag>()
            .map_err(|_| format!("unknown relation tag '{}'", r.rel))?;
        graph.add_relation(Relation::between(tag, NodeId::from(r.from), NodeId::from(r.to)));
    }
    graph.entry = dto.entry.map(NodeId::from);
    Ok(graph)
}

/// Map a UW string to an identity: `00` => null, all-digits => UCL, else UCN.
fn parse_uw(s: &str) -> Uci {
    let s = s.trim();
    if s == "00" {
        Uci::Null
    } else if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        match s.parse::<u64>() {
            Ok(id) => Uci::ucl(id),
            Err(_) => Uci::ucn(s),
        }
    } else {
        Uci::ucn(s)
    }
}

/// Tolerate a model that wraps the JSON in prose or fences: take the substring
/// from the first `{` to the last `}`.
fn extract_json(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b >= a => &raw[a..=b],
        _ => raw,
    }
}

fn diagnostics_summary(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("  - [{:?}] {}", d.code, d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repair_prompt(base: &Prompt, previous: &str, diagnostics: &[Diagnostic]) -> Prompt {
    Prompt {
        system: base.system.clone(),
        user: format!(
            "{}\n\nYour previous output:\n{previous}\n\nThe validator reported these errors:\n{}\n\n\
             Produce a corrected UNL graph JSON that fixes them.",
            base.user,
            diagnostics_summary(diagnostics)
        ),
        format: base.format.clone(),
    }
}

fn parse_repair_prompt(base: &Prompt, previous: &str, error: &str) -> Prompt {
    Prompt {
        system: base.system.clone(),
        user: format!(
            "{}\n\nYour previous output could not be parsed:\n{previous}\n\nError: {error}\n\n\
             Return ONLY a valid UNL graph JSON object.",
            base.user
        ),
        format: base.format.clone(),
    }
}
