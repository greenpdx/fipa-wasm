// content/ontology.rs - Ontology Framework
//
//! Ontology definitions for content validation
//!
//! Provides:
//! - Content element types (Concept, Predicate, Action)
//! - Schema definitions for validation
//! - Built-in FIPA ontologies

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

/// Ontology errors
#[derive(Debug, Error)]
pub enum OntologyError {
    #[error("Unknown concept: {0}")]
    UnknownConcept(String),

    #[error("Unknown predicate: {0}")]
    UnknownPredicate(String),

    #[error("Unknown action: {0}")]
    UnknownAction(String),

    #[error("Missing required slot: {0}")]
    MissingSlot(String),

    #[error("Invalid slot type for '{slot}': expected {expected}, got {actual}")]
    InvalidSlotType {
        slot: String,
        expected: String,
        actual: String,
    },

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Schema not found: {0}")]
    SchemaNotFound(String),
}

/// Term types in content expressions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Term {
    /// String literal
    String(String),
    /// Integer value
    Integer(i64),
    /// Floating point value
    Float(f64),
    /// Boolean value
    Boolean(bool),
    /// Agent identifier
    AgentId(String),
    /// Variable (for queries)
    Variable(String),
    /// Nested concept
    Concept(Box<Concept>),
    /// List of terms
    List(Vec<Term>),
    /// Null/empty value
    Null,
}

impl Term {
    /// Create a string term
    pub fn string(s: &str) -> Self {
        Term::String(s.to_string())
    }

    /// Create an integer term
    pub fn integer(n: i64) -> Self {
        Term::Integer(n)
    }

    /// Create a float term
    pub fn float(f: f64) -> Self {
        Term::Float(f)
    }

    /// Create a boolean term
    pub fn boolean(b: bool) -> Self {
        Term::Boolean(b)
    }

    /// Create an agent ID term
    pub fn agent_id(id: &str) -> Self {
        Term::AgentId(id.to_string())
    }

    /// Create a variable term
    pub fn variable(name: &str) -> Self {
        Term::Variable(name.to_string())
    }

    /// Create a list term
    pub fn list(items: Vec<Term>) -> Self {
        Term::List(items)
    }

    /// Get the type name of this term
    pub fn type_name(&self) -> &'static str {
        match self {
            Term::String(_) => "string",
            Term::Integer(_) => "integer",
            Term::Float(_) => "float",
            Term::Boolean(_) => "boolean",
            Term::AgentId(_) => "agent-id",
            Term::Variable(_) => "variable",
            Term::Concept(_) => "concept",
            Term::List(_) => "list",
            Term::Null => "null",
        }
    }

    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Term::String(s) => Some(s),
            Term::AgentId(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Term::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Term::Float(f) => Some(*f),
            Term::Integer(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// Try to get as boolean
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Term::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

/// A concept (named frame with slots)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    /// Concept name
    pub name: String,
    /// Slots (name -> value pairs)
    pub slots: HashMap<String, Term>,
}

impl Concept {
    /// Create a new concept
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            slots: HashMap::new(),
        }
    }

    /// Add a slot
    pub fn with_slot(mut self, name: &str, value: Term) -> Self {
        self.slots.insert(name.to_string(), value);
        self
    }

    /// Set a slot value
    pub fn set_slot(&mut self, name: &str, value: Term) {
        self.slots.insert(name.to_string(), value);
    }

    /// Get a slot value
    pub fn get_slot(&self, name: &str) -> Option<&Term> {
        self.slots.get(name)
    }

    /// Check if a slot exists
    pub fn has_slot(&self, name: &str) -> bool {
        self.slots.contains_key(name)
    }

    /// Get slot names
    pub fn slot_names(&self) -> Vec<&String> {
        self.slots.keys().collect()
    }
}

/// A predicate (truth statement)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Predicate {
    /// Predicate name
    pub name: String,
    /// Arguments
    pub arguments: Vec<Term>,
}

impl Predicate {
    /// Create a new predicate
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            arguments: vec![],
        }
    }

    /// Add an argument
    pub fn with_arg(mut self, arg: Term) -> Self {
        self.arguments.push(arg);
        self
    }

    /// Add multiple arguments
    pub fn with_args(mut self, args: Vec<Term>) -> Self {
        self.arguments.extend(args);
        self
    }
}

/// An action (performable operation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// Action name
    pub name: String,
    /// Actor (agent performing the action)
    pub actor: Option<String>,
    /// Action arguments (as a concept)
    pub arguments: HashMap<String, Term>,
}

impl Action {
    /// Create a new action
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            actor: None,
            arguments: HashMap::new(),
        }
    }

    /// Set the actor
    pub fn with_actor(mut self, actor: &str) -> Self {
        self.actor = Some(actor.to_string());
        self
    }

    /// Add an argument
    pub fn with_arg(mut self, name: &str, value: Term) -> Self {
        self.arguments.insert(name.to_string(), value);
        self
    }

    /// Get an argument
    pub fn get_arg(&self, name: &str) -> Option<&Term> {
        self.arguments.get(name)
    }
}

/// Content element - the main content type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentElement {
    /// A concept (frame)
    Concept(Concept),
    /// A predicate (proposition)
    Predicate(Predicate),
    /// An action expression
    Action(Action),
    /// A proposition (for inform)
    Proposition(Box<ContentElement>, bool),
    /// Identifying referential expression (iota)
    Iota(String, Box<ContentElement>),
    /// Any referential expression (any)
    Any(String, Box<ContentElement>),
    /// All referential expression (all)
    All(String, Box<ContentElement>),
    /// Sequence of elements
    Sequence(Vec<ContentElement>),
    /// Raw bytes (for unknown content)
    Raw(Vec<u8>),
}

impl ContentElement {
    /// Create a concept element
    pub fn concept(concept: Concept) -> Self {
        ContentElement::Concept(concept)
    }

    /// Create a predicate element
    pub fn predicate(predicate: Predicate) -> Self {
        ContentElement::Predicate(predicate)
    }

    /// Create an action element
    pub fn action(action: Action) -> Self {
        ContentElement::Action(action)
    }

    /// Create a true proposition
    pub fn is_true(content: ContentElement) -> Self {
        ContentElement::Proposition(Box::new(content), true)
    }

    /// Create a false proposition
    pub fn is_false(content: ContentElement) -> Self {
        ContentElement::Proposition(Box::new(content), false)
    }

    /// Create an iota expression (the unique X such that)
    pub fn iota(var: &str, condition: ContentElement) -> Self {
        ContentElement::Iota(var.to_string(), Box::new(condition))
    }

    /// Create a sequence
    pub fn sequence(elements: Vec<ContentElement>) -> Self {
        ContentElement::Sequence(elements)
    }

    /// Get the element type name
    pub fn type_name(&self) -> &'static str {
        match self {
            ContentElement::Concept(_) => "concept",
            ContentElement::Predicate(_) => "predicate",
            ContentElement::Action(_) => "action",
            ContentElement::Proposition(_, _) => "proposition",
            ContentElement::Iota(_, _) => "iota",
            ContentElement::Any(_, _) => "any",
            ContentElement::All(_, _) => "all",
            ContentElement::Sequence(_) => "sequence",
            ContentElement::Raw(_) => "raw",
        }
    }
}

/// Schema field types
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaType {
    String,
    Integer,
    Float,
    Boolean,
    AgentId,
    Concept(String), // Reference to another concept schema
    List(Box<SchemaType>),
    Any,
}

impl SchemaType {
    /// Check if a term matches this type
    pub fn matches(&self, term: &Term) -> bool {
        match (self, term) {
            (SchemaType::String, Term::String(_)) => true,
            (SchemaType::Integer, Term::Integer(_)) => true,
            (SchemaType::Float, Term::Float(_)) => true,
            (SchemaType::Float, Term::Integer(_)) => true, // Allow int as float
            (SchemaType::Boolean, Term::Boolean(_)) => true,
            (SchemaType::AgentId, Term::AgentId(_)) => true,
            (SchemaType::AgentId, Term::String(_)) => true, // Allow string as agent-id
            (SchemaType::Concept(_), Term::Concept(_)) => true,
            (SchemaType::List(inner), Term::List(items)) => {
                items.iter().all(|item| inner.matches(item))
            }
            (SchemaType::Any, _) => true,
            _ => false,
        }
    }

    /// Get type name
    pub fn name(&self) -> String {
        match self {
            SchemaType::String => "string".to_string(),
            SchemaType::Integer => "integer".to_string(),
            SchemaType::Float => "float".to_string(),
            SchemaType::Boolean => "boolean".to_string(),
            SchemaType::AgentId => "agent-id".to_string(),
            SchemaType::Concept(name) => format!("concept:{}", name),
            SchemaType::List(inner) => format!("list<{}>", inner.name()),
            SchemaType::Any => "any".to_string(),
        }
    }
}

/// A schema field definition
#[derive(Debug, Clone)]
pub struct SchemaField {
    /// Field name
    pub name: String,
    /// Field type
    pub field_type: SchemaType,
    /// Is this field required?
    pub required: bool,
    /// Default value (if not required)
    pub default: Option<Term>,
    /// Description
    pub description: String,
}

impl SchemaField {
    /// Create a required field
    pub fn required(name: &str, field_type: SchemaType) -> Self {
        Self {
            name: name.to_string(),
            field_type,
            required: true,
            default: None,
            description: String::new(),
        }
    }

    /// Create an optional field
    pub fn optional(name: &str, field_type: SchemaType) -> Self {
        Self {
            name: name.to_string(),
            field_type,
            required: false,
            default: None,
            description: String::new(),
        }
    }

    /// Add a description
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    /// Add a default value
    pub fn with_default(mut self, value: Term) -> Self {
        self.default = Some(value);
        self
    }
}

/// Schema for a concept, predicate, or action
#[derive(Debug, Clone)]
pub struct Schema {
    /// Schema name
    pub name: String,
    /// Schema kind
    pub kind: SchemaKind,
    /// Fields/slots
    pub fields: Vec<SchemaField>,
    /// Description
    pub description: String,
    /// Parent schema (for inheritance)
    pub parent: Option<String>,
}

/// Schema kinds
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaKind {
    Concept,
    Predicate,
    Action,
}

impl Schema {
    /// Create a new concept schema
    pub fn concept(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SchemaKind::Concept,
            fields: vec![],
            description: String::new(),
            parent: None,
        }
    }

    /// Create a new predicate schema
    pub fn predicate(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SchemaKind::Predicate,
            fields: vec![],
            description: String::new(),
            parent: None,
        }
    }

    /// Create a new action schema
    pub fn action(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SchemaKind::Action,
            fields: vec![],
            description: String::new(),
            parent: None,
        }
    }

    /// Add a field
    pub fn with_field(mut self, field: SchemaField) -> Self {
        self.fields.push(field);
        self
    }

    /// Add description
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    /// Set parent schema
    pub fn extends(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_string());
        self
    }

    /// Get a field by name
    pub fn get_field(&self, name: &str) -> Option<&SchemaField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Validate a concept against this schema
    pub fn validate_concept(&self, concept: &Concept) -> Result<(), OntologyError> {
        if self.kind != SchemaKind::Concept {
            return Err(OntologyError::ValidationFailed(format!(
                "Schema '{}' is not a concept schema",
                self.name
            )));
        }

        // Check required fields
        for field in &self.fields {
            if field.required {
                if !concept.has_slot(&field.name) {
                    return Err(OntologyError::MissingSlot(field.name.clone()));
                }
            }

            // Check field types
            if let Some(value) = concept.get_slot(&field.name) {
                if !field.field_type.matches(value) {
                    return Err(OntologyError::InvalidSlotType {
                        slot: field.name.clone(),
                        expected: field.field_type.name(),
                        actual: value.type_name().to_string(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Ontology trait
pub trait Ontology: Send + Sync {
    /// Get the ontology name
    fn name(&self) -> &str;

    /// Get the ontology version
    fn version(&self) -> &str {
        "1.0"
    }

    /// Validate a content element
    fn validate(&self, element: &ContentElement) -> Result<(), OntologyError>;

    /// Get a schema by name
    fn get_schema(&self, name: &str) -> Option<&Schema>;

    /// List all schema names
    fn list_schemas(&self) -> Vec<&str>;
}

/// Registry of ontologies
#[derive(Default)]
pub struct OntologyRegistry {
    ontologies: HashMap<String, Arc<dyn Ontology>>,
}

impl OntologyRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            ontologies: HashMap::new(),
        }
    }

    /// Register an ontology
    pub fn register(&mut self, ontology: Arc<dyn Ontology>) {
        let name = ontology.name().to_string();
        self.ontologies.insert(name, ontology);
    }

    /// Get an ontology by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Ontology>> {
        self.ontologies.get(name).cloned()
    }

    /// List registered ontologies
    pub fn list(&self) -> Vec<String> {
        self.ontologies.keys().cloned().collect()
    }
}

// ============================================================================
// Built-in Ontologies
// ============================================================================

/// FIPA Agent Management Ontology
pub struct FipaAgentManagementOntology {
    schemas: HashMap<String, Schema>,
}

impl FipaAgentManagementOntology {
    pub fn new() -> Self {
        let mut schemas = HashMap::new();

        // Agent Description
        schemas.insert(
            "agent-description".to_string(),
            Schema::concept("agent-description")
                .with_description("Description of an agent")
                .with_field(SchemaField::required("name", SchemaType::AgentId))
                .with_field(SchemaField::optional("services", SchemaType::List(Box::new(SchemaType::Concept("service-description".to_string())))))
                .with_field(SchemaField::optional("protocols", SchemaType::List(Box::new(SchemaType::String))))
                .with_field(SchemaField::optional("languages", SchemaType::List(Box::new(SchemaType::String)))),
        );

        // Service Description
        schemas.insert(
            "service-description".to_string(),
            Schema::concept("service-description")
                .with_description("Description of a service")
                .with_field(SchemaField::required("name", SchemaType::String))
                .with_field(SchemaField::optional("type", SchemaType::String))
                .with_field(SchemaField::optional("protocols", SchemaType::List(Box::new(SchemaType::String))))
                .with_field(SchemaField::optional("ontologies", SchemaType::List(Box::new(SchemaType::String))))
                .with_field(SchemaField::optional("properties", SchemaType::Any)),
        );

        // Platform Description
        schemas.insert(
            "platform-description".to_string(),
            Schema::concept("platform-description")
                .with_description("Description of an agent platform")
                .with_field(SchemaField::required("name", SchemaType::String))
                .with_field(SchemaField::optional("addresses", SchemaType::List(Box::new(SchemaType::String)))),
        );

        // Register action
        schemas.insert(
            "register".to_string(),
            Schema::action("register")
                .with_description("Register an agent or service")
                .with_field(SchemaField::required("description", SchemaType::Concept("agent-description".to_string()))),
        );

        // Deregister action
        schemas.insert(
            "deregister".to_string(),
            Schema::action("deregister")
                .with_description("Deregister an agent or service")
                .with_field(SchemaField::required("description", SchemaType::Concept("agent-description".to_string()))),
        );

        // Search action
        schemas.insert(
            "search".to_string(),
            Schema::action("search")
                .with_description("Search for agents or services")
                .with_field(SchemaField::required("description", SchemaType::Concept("agent-description".to_string())))
                .with_field(SchemaField::optional("max-results", SchemaType::Integer)),
        );

        // Modify action
        schemas.insert(
            "modify".to_string(),
            Schema::action("modify")
                .with_description("Modify an agent or service registration")
                .with_field(SchemaField::required("description", SchemaType::Concept("agent-description".to_string()))),
        );

        Self { schemas }
    }
}

impl Default for FipaAgentManagementOntology {
    fn default() -> Self {
        Self::new()
    }
}

impl Ontology for FipaAgentManagementOntology {
    fn name(&self) -> &str {
        "fipa-agent-management"
    }

    fn validate(&self, element: &ContentElement) -> Result<(), OntologyError> {
        match element {
            ContentElement::Concept(concept) => {
                if let Some(schema) = self.schemas.get(&concept.name) {
                    schema.validate_concept(concept)
                } else {
                    // Unknown concepts are allowed (extensible)
                    Ok(())
                }
            }
            ContentElement::Action(action) => {
                if let Some(schema) = self.schemas.get(&action.name) {
                    // Validate action arguments
                    for field in &schema.fields {
                        if field.required && !action.arguments.contains_key(&field.name) {
                            return Err(OntologyError::MissingSlot(field.name.clone()));
                        }
                    }
                    Ok(())
                } else {
                    Err(OntologyError::UnknownAction(action.name.clone()))
                }
            }
            ContentElement::Sequence(elements) => {
                for elem in elements {
                    self.validate(elem)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn get_schema(&self, name: &str) -> Option<&Schema> {
        self.schemas.get(name)
    }

    fn list_schemas(&self) -> Vec<&str> {
        self.schemas.keys().map(|s| s.as_str()).collect()
    }
}

/// Create FIPA Agent Management ontology
pub fn fipa_agent_management() -> FipaAgentManagementOntology {
    FipaAgentManagementOntology::new()
}

/// FIPA Ping Ontology (for testing)
pub struct FipaPingOntology {
    schemas: HashMap<String, Schema>,
}

impl FipaPingOntology {
    pub fn new() -> Self {
        let mut schemas = HashMap::new();

        // Ping action
        schemas.insert(
            "ping".to_string(),
            Schema::action("ping")
                .with_description("Ping another agent"),
        );

        // Pong predicate
        schemas.insert(
            "alive".to_string(),
            Schema::predicate("alive")
                .with_description("Agent is alive")
                .with_field(SchemaField::required("agent", SchemaType::AgentId)),
        );

        Self { schemas }
    }
}

impl Default for FipaPingOntology {
    fn default() -> Self {
        Self::new()
    }
}

impl Ontology for FipaPingOntology {
    fn name(&self) -> &str {
        "fipa-ping"
    }

    fn validate(&self, element: &ContentElement) -> Result<(), OntologyError> {
        match element {
            ContentElement::Action(action) => {
                if action.name == "ping" {
                    Ok(())
                } else {
                    Err(OntologyError::UnknownAction(action.name.clone()))
                }
            }
            ContentElement::Predicate(pred) => {
                if pred.name == "alive" {
                    if pred.arguments.is_empty() {
                        Err(OntologyError::MissingSlot("agent".to_string()))
                    } else {
                        Ok(())
                    }
                } else {
                    Err(OntologyError::UnknownPredicate(pred.name.clone()))
                }
            }
            _ => Ok(()),
        }
    }

    fn get_schema(&self, name: &str) -> Option<&Schema> {
        self.schemas.get(name)
    }

    fn list_schemas(&self) -> Vec<&str> {
        self.schemas.keys().map(|s| s.as_str()).collect()
    }
}

/// Create FIPA Ping ontology
pub fn fipa_ping() -> FipaPingOntology {
    FipaPingOntology::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_types() {
        assert_eq!(Term::string("test").type_name(), "string");
        assert_eq!(Term::integer(42).type_name(), "integer");
        assert_eq!(Term::boolean(true).type_name(), "boolean");
    }

    #[test]
    fn test_concept_creation() {
        let concept = Concept::new("agent-description")
            .with_slot("name", Term::string("my-agent"))
            .with_slot("priority", Term::integer(5));

        assert_eq!(concept.name, "agent-description");
        assert!(concept.has_slot("name"));
        assert!(concept.has_slot("priority"));
        assert!(!concept.has_slot("unknown"));
    }

    #[test]
    fn test_schema_validation() {
        let schema = Schema::concept("test")
            .with_field(SchemaField::required("name", SchemaType::String))
            .with_field(SchemaField::optional("count", SchemaType::Integer));

        // Valid concept
        let valid = Concept::new("test")
            .with_slot("name", Term::string("value"));
        assert!(schema.validate_concept(&valid).is_ok());

        // Missing required field
        let invalid = Concept::new("test");
        assert!(matches!(
            schema.validate_concept(&invalid),
            Err(OntologyError::MissingSlot(_))
        ));

        // Wrong type
        let wrong_type = Concept::new("test")
            .with_slot("name", Term::integer(42));
        assert!(matches!(
            schema.validate_concept(&wrong_type),
            Err(OntologyError::InvalidSlotType { .. })
        ));
    }

    #[test]
    fn test_fipa_agent_management() {
        let ontology = fipa_agent_management();

        assert_eq!(ontology.name(), "fipa-agent-management");
        assert!(ontology.get_schema("agent-description").is_some());
        assert!(ontology.get_schema("service-description").is_some());
        assert!(ontology.get_schema("register").is_some());
    }

    #[test]
    fn test_fipa_ping() {
        let ontology = fipa_ping();

        assert_eq!(ontology.name(), "fipa-ping");

        // Valid ping action
        let ping = ContentElement::action(Action::new("ping"));
        assert!(ontology.validate(&ping).is_ok());

        // Valid alive predicate
        let alive = ContentElement::predicate(
            Predicate::new("alive").with_arg(Term::agent_id("agent1"))
        );
        assert!(ontology.validate(&alive).is_ok());
    }

    #[test]
    fn test_ontology_registry() {
        let mut registry = OntologyRegistry::new();
        registry.register(Arc::new(fipa_agent_management()));
        registry.register(Arc::new(fipa_ping()));

        assert_eq!(registry.list().len(), 2);
        assert!(registry.get("fipa-agent-management").is_some());
        assert!(registry.get("fipa-ping").is_some());
    }
}
