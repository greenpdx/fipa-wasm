//! Pipeline tests using a mock reasoning backend (canned responses), so the
//! decode → validate → repair logic is exercised without a live model.

use crate::*;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex;
use unl_core::{Attr, Lang, NodeId, NodeRef, RelationTag, Uci};
use unl_kb::MemKb;
use unl_validator::{DiagCode, Severity};

/// Returns canned responses in order; errors once exhausted.
struct MockBackend {
    responses: Mutex<VecDeque<String>>,
}

impl MockBackend {
    fn new(responses: Vec<&str>) -> Self {
        MockBackend {
            responses: Mutex::new(responses.into_iter().map(String::from).collect()),
        }
    }
}

#[async_trait]
impl ReasoningBackend for MockBackend {
    async fn complete(&self, _prompt: &Prompt) -> Result<String, LlmError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| LlmError::Backend("no more canned responses".into()))
    }
}

fn kb() -> MemKb {
    MemKb::from_toml(include_str!("../../../data/kb-seed/memkb-fixture.toml")).unwrap()
}

#[test]
fn reasoning_backend_is_object_safe() {
    let mock = MockBackend::new(vec![]);
    let _dyn_backend: &dyn ReasoningBackend = &mock;
}

#[tokio::test]
async fn clean_unlization_validates_against_kb() {
    // cat icl animal — both resolve and is_a holds, so zero diagnostics.
    let backend = MockBackend::new(vec![
        r#"{"nodes":[{"id":"01","uw":"cat"},{"id":"02","uw":"animal"}],
            "relations":[{"rel":"icl","from":"01","to":"02"}],"entry":"01"}"#,
    ]);
    let unlizer = LlmUnlizer::new(backend, kb());
    let out = unlizer.unlize("a cat is an animal", Lang::ENG).await.unwrap();

    assert!(out.residual_diagnostics.is_empty(), "{:?}", out.residual_diagnostics);
    assert_eq!(out.confidence, 1.0);
    assert_eq!(out.graph.nodes.len(), 2);
    assert_eq!(out.graph.relations[0].tag, RelationTag::Icl);
    assert_eq!(out.graph.entry, Some("01".into()));
}

#[tokio::test]
async fn tolerates_prose_wrapped_json() {
    let backend = MockBackend::new(vec![
        "Here is the graph:\n```json\n{\"nodes\":[{\"id\":\"01\",\"uw\":\"cat\"}],\
         \"relations\":[],\"entry\":\"01\"}\n```\nDone.",
    ]);
    let out = LlmUnlizer::new(backend, kb())
        .unlize("cat", Lang::ENG)
        .await
        .unwrap();
    assert_eq!(out.graph.nodes.len(), 1);
}

#[tokio::test]
async fn unknown_concepts_lower_confidence_but_do_not_block() {
    // kill/Peter/John are not in the fixture KB => UnknownConcept warnings.
    let backend = MockBackend::new(vec![
        r#"{"nodes":[{"id":"01","uw":"kill","attrs":["past"]},
            {"id":"02","uw":"Peter"},{"id":"03","uw":"John"}],
            "relations":[{"rel":"agt","from":"01","to":"02"},
                         {"rel":"obj","from":"01","to":"03"}],"entry":"01"}"#,
    ]);
    let out = LlmUnlizer::new(backend, kb())
        .unlize("Peter killed John", Lang::ENG)
        .await
        .unwrap();

    assert!(out.residual_diagnostics.iter().all(|d| d.severity == Severity::Warning));
    assert!(out
        .residual_diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnknownConcept));
    assert!(out.confidence < 0.5, "confidence was {}", out.confidence);
    // Relations reference declared nodes by id, and the @past attribute survived.
    assert!(matches!(out.graph.relations[0].source, NodeRef::Id(_)));
    assert!(out.graph.nodes.get(&NodeId::from("01")).unwrap().attributes.contains(&Attr::Past));
}

#[tokio::test]
async fn unparseable_tag_triggers_repair() {
    // First output uses a fabricated relation tag => decode error => repair.
    let backend = MockBackend::new(vec![
        r#"{"nodes":[{"id":"01","uw":"cat"},{"id":"02","uw":"animal"}],
            "relations":[{"rel":"isakindof","from":"01","to":"02"}]}"#,
        r#"{"nodes":[{"id":"01","uw":"cat"},{"id":"02","uw":"animal"}],
            "relations":[{"rel":"icl","from":"01","to":"02"}],"entry":"01"}"#,
    ]);
    let out = LlmUnlizer::new(backend, kb())
        .unlize("a cat is an animal", Lang::ENG)
        .await
        .unwrap();
    assert_eq!(out.graph.relations[0].tag, RelationTag::Icl);
}

#[tokio::test]
async fn dangling_reference_triggers_repair() {
    // First output references a node that doesn't exist => Error => repair.
    let backend = MockBackend::new(vec![
        r#"{"nodes":[{"id":"01","uw":"cat"}],
            "relations":[{"rel":"icl","from":"01","to":"99"}],"entry":"01"}"#,
        r#"{"nodes":[{"id":"01","uw":"cat"},{"id":"02","uw":"animal"}],
            "relations":[{"rel":"icl","from":"01","to":"02"}],"entry":"01"}"#,
    ]);
    let out = LlmUnlizer::new(backend, kb())
        .unlize("a cat is an animal", Lang::ENG)
        .await
        .unwrap();
    assert!(!out
        .residual_diagnostics
        .iter()
        .any(|d| d.code == DiagCode::DanglingReference));
    assert_eq!(out.graph.nodes.len(), 2);
}

#[tokio::test]
async fn exhausted_repairs_returns_structured_error() {
    let backend = MockBackend::new(vec!["not json at all", "still not json"]);
    let err = LlmUnlizer::new(backend, kb())
        .with_max_repairs(1)
        .unlize("whatever", Lang::ENG)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)));
}

#[tokio::test]
async fn null_proform_decodes() {
    let backend = MockBackend::new(vec![
        r#"{"nodes":[{"id":"01","uw":"stop","attrs":["past","negative"]},
            {"id":"02","uw":"00"}],
            "relations":[{"rel":"agt","from":"01","to":"02"}],"entry":"01"}"#,
    ]);
    let out = LlmUnlizer::new(backend, kb())
        .unlize("it did not stop", Lang::ENG)
        .await
        .unwrap();
    assert_eq!(out.graph.nodes.get(&NodeId::from("02")).unwrap().uci, Uci::Null);
    // The 00 pro-form is flagged for completeness.
    assert!(out
        .residual_diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnsaturatedProForm));
}
