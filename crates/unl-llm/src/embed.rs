//! Embedding-assisted grounding — the "vector DB" lever for UNLization quality.
//!
//! Exact lemma lookup ([`unl_kb::KnowledgeBase::candidates`]) is brittle:
//! morphological variants, multiword expressions, and sense ambiguity all slip
//! through. An embedding index retrieves concepts by *meaning* — fuzzy
//! candidates plus context-based sense ranking *before* the LLM runs.
//!
//! This is additive: a [`SemanticGrounder`] plugged into [`crate::LlmUnlizer`]
//! augments (does not replace) the exact KB grounding. [`VectorIndex`] is the
//! in-memory store; [`OllamaEmbedder`] embeds via a local Ollama model, keeping
//! the edge/CPU-only posture; any other embedder is a second [`Embedder`] impl.

use crate::LlmError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use unl_core::Uci;

/// Produces an embedding vector for a piece of text.
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError>;
}

/// An in-memory vector store of `(concept, embedding)` with cosine nearest-
/// neighbour search — the "vector DB". A persistent/ANN backend (FAISS, qdrant,
/// …) is a drop-in replacement behind the same surface.
#[derive(Default)]
pub struct VectorIndex {
    entries: Vec<(Uci, Vec<f32>)>,
}

impl VectorIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, concept: Uci, embedding: Vec<f32>) {
        self.entries.push((concept, embedding));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The `k` concepts whose embeddings are most cosine-similar to `query`,
    /// best first.
    pub fn nearest(&self, query: &[f32], k: usize) -> Vec<(Uci, f32)> {
        let mut scored: Vec<(Uci, f32)> = self
            .entries
            .iter()
            .map(|(c, v)| (c.clone(), cosine(query, v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Retrieves concepts semantically related to a context string.
#[async_trait]
pub trait SemanticGrounder: Send + Sync {
    async fn related_concepts(&self, context: &str, k: usize) -> Result<Vec<Uci>, LlmError>;
}

/// The default grounder: embed the context, return the index's nearest concepts.
pub struct VectorGrounder<E: Embedder> {
    embedder: E,
    index: VectorIndex,
}

impl<E: Embedder> VectorGrounder<E> {
    pub fn new(embedder: E, index: VectorIndex) -> Self {
        VectorGrounder { embedder, index }
    }
}

#[async_trait]
impl<E: Embedder> SemanticGrounder for VectorGrounder<E> {
    async fn related_concepts(&self, context: &str, k: usize) -> Result<Vec<Uci>, LlmError> {
        let query = self.embedder.embed(context).await?;
        Ok(self
            .index
            .nearest(&query, k)
            .into_iter()
            .map(|(c, _)| c)
            .collect())
    }
}

/// Embeds via a local Ollama model (`/api/embed`).
pub struct OllamaEmbedder {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaEmbedder {
    pub fn new(model: impl Into<String>) -> Self {
        OllamaEmbedder {
            client: reqwest::Client::new(),
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            input: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            embeddings: Vec<Vec<f32>>,
        }
        let resp = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&Req {
                model: &self.model,
                input: text,
            })
            .send()
            .await?
            .error_for_status()
            .map_err(|e| LlmError::Backend(e.to_string()))?;
        let parsed: Resp = resp.json().await?;
        parsed
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Backend("empty embedding response".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic embedder: a 3-dim vector keyed off the first character, so
    /// related words land near each other without a live model.
    struct MockEmbedder;

    #[async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
            let c = text.chars().next().unwrap_or('a') as u32 as f32;
            Ok(vec![c, (c * 2.0) % 7.0, (c * 3.0) % 5.0])
        }
    }

    #[test]
    fn nearest_ranks_by_cosine() {
        let mut idx = VectorIndex::new();
        idx.insert(Uci::ucl(1), vec![1.0, 0.0, 0.0]);
        idx.insert(Uci::ucl(2), vec![0.0, 1.0, 0.0]);
        idx.insert(Uci::ucl(3), vec![0.9, 0.1, 0.0]);
        let near = idx.nearest(&[1.0, 0.0, 0.0], 2);
        assert_eq!(near.len(), 2);
        assert_eq!(near[0].0, Uci::ucl(1)); // identical direction
        assert_eq!(near[1].0, Uci::ucl(3)); // closest of the rest
        assert!(near[0].1 > near[1].1);
    }

    #[test]
    fn empty_vectors_score_zero() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[tokio::test]
    async fn vector_grounder_returns_nearest_concepts() {
        let mut idx = VectorIndex::new();
        // "cat" and "cot" share a first char => near; "dog" differs.
        idx.insert(Uci::ucn("cat"), MockEmbedder.embed("cat").await.unwrap());
        idx.insert(Uci::ucn("dog"), MockEmbedder.embed("dog").await.unwrap());
        let grounder = VectorGrounder::new(MockEmbedder, idx);
        let related = grounder.related_concepts("cot", 1).await.unwrap();
        assert_eq!(related, vec![Uci::ucn("cat")]);
    }

    #[test]
    fn semantic_grounder_is_object_safe() {
        let g = VectorGrounder::new(MockEmbedder, VectorIndex::new());
        let _dyn_g: &dyn SemanticGrounder = &g;
    }
}
