# Future Enhancements: Ontology, Local NLP, and MCP Integration

This document outlines three major enhancements planned for the FIPA WASM Agent System.

---

## 1. Enhanced Ontology and ACL

### Current Limitations

| Area | Current State | Limitation |
|------|---------------|------------|
| Ontology | Custom schema validation | No semantic reasoning, no inheritance |
| ACL Content | FIPA-SL codec | Limited expressiveness |
| Interoperability | Custom format | Not compatible with Semantic Web standards |
| Scalability | Direct validation | No caching, no inference |

### Proposed Improvements

#### 1.1 Semantic Web Integration (OWL/RDF)

Add W3C standard ontology support using the [horned-owl](https://github.com/phillord/horned-owl) Rust library.

```rust
use horned_owl::model::*;
use horned_owl::io::owx::reader::read;

/// Enhanced ontology with OWL2 support
pub trait SemanticOntology {
    /// Load ontology from OWL file
    fn load_owl(&mut self, path: &Path) -> Result<(), OntologyError>;

    /// Perform reasoning and return inferences
    fn reason(&self, content: &ContentElement) -> Vec<Inference>;

    /// Check if one class subsumes another
    fn check_subsumption(&self, sub: &str, super_class: &str) -> bool;

    /// Execute SPARQL query
    fn sparql_query(&self, query: &str) -> QueryResult;
}

/// Ontology with inheritance support
pub struct OwlOntology {
    classes: HashMap<String, OwlClass>,
    properties: HashMap<String, OwlProperty>,
    individuals: HashMap<String, Individual>,
    axioms: Vec<Axiom>,
}

impl OwlOntology {
    /// Check if instance satisfies class constraints
    pub fn validate_instance(&self, instance: &ContentElement, class: &str)
        -> Result<(), ValidationError>
    {
        let class_def = self.classes.get(class)
            .ok_or(ValidationError::UnknownClass(class.into()))?;

        // Check all parent classes (inheritance)
        for parent in &class_def.parents {
            self.validate_instance(instance, parent)?;
        }

        // Check property restrictions
        for restriction in &class_def.restrictions {
            self.check_restriction(instance, restriction)?;
        }

        Ok(())
    }
}
```

**Benefits:**
- Ontology inheritance and class hierarchies
- Automatic inference and reasoning
- Interoperability with existing ontologies (FOAF, Dublin Core, Schema.org)
- SPARQL queries over agent knowledge bases

#### 1.2 JSON-LD Content Language

Add JSON-LD as a modern, developer-friendly alternative to FIPA-SL.

```rust
use json_ld::{JsonLdProcessor, RemoteDocument};

/// JSON-LD codec for content encoding
pub struct JsonLdCodec {
    context: serde_json::Value,
    processor: JsonLdProcessor,
}

impl Codec for JsonLdCodec {
    fn language(&self) -> &str {
        "application/ld+json"
    }

    fn encode(&self, content: &ContentElement) -> Result<Vec<u8>, CodecError> {
        let mut doc = serde_json::to_value(content)?;
        doc["@context"] = self.context.clone();
        Ok(serde_json::to_vec(&doc)?)
    }

    fn decode(&self, bytes: &[u8]) -> Result<ContentElement, CodecError> {
        let doc: serde_json::Value = serde_json::from_slice(bytes)?;
        // Expand JSON-LD and convert to ContentElement
        let expanded = self.processor.expand(&doc)?;
        ContentElement::from_json_ld(&expanded)
    }
}
```

**Example JSON-LD Message Content:**
```json
{
  "@context": {
    "@vocab": "https://fipa.org/ontology/",
    "xsd": "http://www.w3.org/2001/XMLSchema#"
  },
  "@type": "WeatherReport",
  "location": "Seattle",
  "temperature": {
    "@value": "72",
    "@type": "xsd:integer"
  },
  "conditions": "sunny",
  "timestamp": {
    "@value": "2024-12-28T10:30:00Z",
    "@type": "xsd:dateTime"
  }
}
```

**Benefits:**
- Developer-friendly JSON format
- Self-describing with embedded context
- Linked Data compatibility
- Easy integration with REST APIs and web services

#### 1.3 Enhanced Schema Constraints

Extend the current schema system with richer validation.

```rust
/// Enhanced schema with constraints and inheritance
pub struct EnhancedSchema {
    pub name: String,
    pub parent: Option<String>,           // Inheritance
    pub fields: Vec<SchemaField>,
    pub constraints: Vec<Constraint>,
    pub axioms: Vec<Axiom>,
}

/// Value constraints for fields
pub enum Constraint {
    /// Numeric range
    Range { min: Option<f64>, max: Option<f64> },

    /// Regex pattern
    Pattern(String),

    /// Enumeration of allowed values
    OneOf(Vec<Term>),

    /// Cardinality (min/max occurrences)
    Cardinality { min: usize, max: Option<usize> },

    /// String length
    Length { min: Option<usize>, max: Option<usize> },

    /// Custom validation function
    Custom {
        name: String,
        validator: Box<dyn Fn(&Term) -> bool + Send + Sync>,
    },
}

/// Logical axioms for advanced reasoning
pub enum Axiom {
    /// If A then B
    Implication { antecedent: String, consequent: String },

    /// A and B cannot both be true
    Disjoint { classes: Vec<String> },

    /// A is equivalent to B
    Equivalent { class_a: String, class_b: String },

    /// Property is functional (single value)
    Functional { property: String },

    /// Property is inverse of another
    Inverse { property: String, inverse: String },
}
```

#### 1.4 Enhanced ACL with Semantic Extensions

```protobuf
// Enhanced ACL message with semantic annotations
message EnhancedAclMessage {
    // Standard FIPA fields
    string message_id = 1;
    Performative performative = 2;
    AgentId sender = 3;
    repeated AgentId receivers = 4;
    // ... other standard fields

    // Semantic extensions
    SemanticContext semantic_context = 20;
    repeated Commitment commitments = 21;
    repeated Obligation obligations = 22;
    TrustAssertion trust_level = 23;
}

message SemanticContext {
    // Primary ontology URI
    string ontology_uri = 1;

    // Imported/referenced ontologies
    repeated string imported_ontologies = 2;

    // Namespace prefixes for compact representation
    map<string, string> namespace_prefixes = 3;

    // Content language (fipa-sl, json-ld, etc.)
    string content_language = 4;
}

message Commitment {
    string debtor = 1;      // Agent making commitment
    string creditor = 2;    // Agent receiving commitment
    string content = 3;     // What is committed
    int64 deadline = 4;     // When it must be fulfilled
}
```

#### 1.5 Dependencies

```toml
[dependencies]
# Semantic Web
horned-owl = "1.0"           # OWL2 ontology support
json-ld = "0.16"             # JSON-LD processing
sophia = "0.8"               # RDF toolkit
oxigraph = "0.4"             # SPARQL database (optional)
```

---

## 2. Local NLP Translation

### Overview

Enable natural language processing for ontology translation and content understanding without external API calls. This provides privacy, low latency, and offline capability.

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Agent Platform                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐         │
│  │   Agent A   │    │   Agent B   │    │   Agent C   │         │
│  │ Ontology: X │    │ Ontology: Y │    │ Ontology: Z │         │
│  └──────┬──────┘    └──────┬──────┘    └──────┬──────┘         │
│         │                  │                  │                 │
│         └────────┬─────────┴─────────┬───────┘                 │
│                  ▼                   ▼                          │
│         ┌────────────────────────────────────┐                 │
│         │      Local NLP Translator          │                 │
│         │  ┌──────────────────────────────┐  │                 │
│         │  │  Quantized LLM (Phi-3/Llama) │  │                 │
│         │  │     ~1-4GB RAM, CPU/GPU      │  │                 │
│         │  └──────────────────────────────┘  │                 │
│         └────────────────────────────────────┘                 │
└─────────────────────────────────────────────────────────────────┘
                      No Internet Required
```

### Rust Libraries

| Library | Description | Use Case |
|---------|-------------|----------|
| [Candle](https://github.com/huggingface/candle) | HuggingFace Rust ML | Pure Rust, WASM support |
| [llama-cpp-rs](https://github.com/ggml-org/llama.cpp) | llama.cpp bindings | Best performance |
| [mistral.rs](https://github.com/EricLBuehler/mistral.rs) | Candle wrapper | Easy integration |

### Recommended Models

| Model | Size | RAM | Speed | Use Case |
|-------|------|-----|-------|----------|
| Phi-3-mini | 3.8B | ~3GB | Fast | General translation |
| Llama-3.2-1B | 1B | ~1GB | Very fast | Simple extraction |
| Qwen2.5-1.5B | 1.5B | ~1.5GB | Fast | Multilingual |
| TinyLlama | 1.1B | ~1GB | Very fast | Resource-constrained |

### Implementation

```rust
use candle_core::{Device, Tensor, DType};
use candle_transformers::models::phi3;
use tokenizers::Tokenizer;

/// Local NLP translator - fully offline
pub struct LocalNlpTranslator {
    model: phi3::Model,
    tokenizer: Tokenizer,
    device: Device,
    config: NlpConfig,
}

pub struct NlpConfig {
    /// Model path (GGUF or safetensors)
    pub model_path: PathBuf,

    /// Use GPU if available
    pub use_gpu: bool,

    /// Maximum tokens to generate
    pub max_tokens: usize,

    /// Temperature for generation
    pub temperature: f64,
}

impl LocalNlpTranslator {
    /// Load a quantized model
    pub fn load(config: NlpConfig) -> Result<Self, NlpError> {
        let device = if config.use_gpu {
            Device::cuda_if_available(0)?
        } else {
            Device::Cpu
        };

        let tokenizer = Tokenizer::from_file(&config.model_path.join("tokenizer.json"))?;
        let model = phi3::Model::load(&config.model_path, &device)?;

        Ok(Self { model, tokenizer, device, config })
    }

    /// Translate natural language to structured ontology content
    pub fn nl_to_ontology(
        &self,
        text: &str,
        target_ontology: &Ontology
    ) -> Result<ContentElement, NlpError> {
        let prompt = format!(
            "Extract structured data from the following text.\n\
             Text: \"{}\"\n\n\
             Available concepts: {}\n\
             Available predicates: {}\n\n\
             Output as JSON with @type field:",
            text,
            target_ontology.concept_names().join(", "),
            target_ontology.predicate_names().join(", ")
        );

        let response = self.generate(&prompt)?;
        self.parse_json_response(&response, target_ontology)
    }

    /// Translate content between different ontologies
    pub fn translate_ontology(
        &self,
        content: &ContentElement,
        source: &Ontology,
        target: &Ontology,
    ) -> Result<ContentElement, NlpError> {
        let prompt = format!(
            "Translate the following concept from {} ontology to {} ontology.\n\n\
             Source: {}\n\n\
             Target ontology concepts: {}\n\n\
             Output the equivalent concept in the target ontology as JSON:",
            source.name(),
            target.name(),
            serde_json::to_string_pretty(content)?,
            target.concept_names().join(", ")
        );

        let response = self.generate(&prompt)?;
        self.parse_json_response(&response, target)
    }

    /// Parse natural language ACL message
    pub fn parse_nl_message(&self, text: &str) -> Result<AclMessage, NlpError> {
        let prompt = format!(
            "Parse the following natural language into a FIPA ACL message.\n\n\
             Text: \"{}\"\n\n\
             Output JSON with fields: performative, receiver, content, protocol",
            text
        );

        let response = self.generate(&prompt)?;
        serde_json::from_str(&response).map_err(NlpError::ParseError)
    }

    /// Internal generation function
    fn generate(&self, prompt: &str) -> Result<String, NlpError> {
        let tokens = self.tokenizer.encode(prompt, true)?;
        let input = Tensor::new(tokens.get_ids(), &self.device)?;

        let mut output_tokens = Vec::new();
        let mut logits = self.model.forward(&input, 0)?;

        for _ in 0..self.config.max_tokens {
            let next_token = self.sample(&logits)?;
            if next_token == self.tokenizer.token_to_id("</s>").unwrap() {
                break;
            }
            output_tokens.push(next_token);
            logits = self.model.forward(&Tensor::new(&[next_token], &self.device)?, 0)?;
        }

        Ok(self.tokenizer.decode(&output_tokens, true)?)
    }
}
```

### Alternative: Rule-Based Translation (No LLM)

For simpler cases without LLM overhead:

```rust
/// Lightweight ontology mapper using similarity matching
pub struct RuleBasedTranslator {
    /// Direct concept mappings between ontologies
    mappings: HashMap<(String, String), ConceptMapping>,

    /// Synonym dictionary
    synonyms: HashMap<String, Vec<String>>,

    /// String similarity threshold
    similarity_threshold: f64,
}

impl RuleBasedTranslator {
    /// Load mappings from configuration file
    pub fn load(config_path: &Path) -> Result<Self, Error> {
        let config: TranslatorConfig = serde_json::from_reader(
            File::open(config_path)?
        )?;

        Ok(Self {
            mappings: config.mappings.into_iter().collect(),
            synonyms: config.synonyms,
            similarity_threshold: config.threshold.unwrap_or(0.8),
        })
    }

    /// Find similar concept using string similarity
    pub fn find_similar(
        &self,
        concept: &str,
        ontology: &Ontology
    ) -> Option<(String, f64)> {
        let mut best_match = None;
        let mut best_score = 0.0;

        for name in ontology.concept_names() {
            let score = self.similarity(concept, &name);
            if score > best_score && score >= self.similarity_threshold {
                best_score = score;
                best_match = Some(name);
            }
        }

        best_match.map(|m| (m, best_score))
    }

    /// Jaro-Winkler string similarity
    fn similarity(&self, a: &str, b: &str) -> f64 {
        strsim::jaro_winkler(
            &a.to_lowercase(),
            &b.to_lowercase()
        )
    }
}
```

### Dependencies

```toml
[dependencies]
# Option 1: Candle (pure Rust, WASM compatible)
candle-core = "0.8"
candle-nn = "0.8"
candle-transformers = "0.8"
hf-hub = "0.3"              # Download models from HuggingFace
tokenizers = "0.20"

# Option 2: llama.cpp bindings (fastest inference)
llama-cpp-2 = "0.1"

# String similarity for rule-based fallback
strsim = "0.11"
```

### Comparison: Local vs Cloud

| Aspect | Local LLM | Cloud API |
|--------|-----------|-----------|
| Privacy | Full data privacy | Data sent externally |
| Latency | ~50-200ms | ~500-2000ms |
| Cost | One-time download | Per-token pricing |
| Offline | Works offline | Requires internet |
| Quality | Good (small models) | Best (large models) |
| RAM | 1-4GB | None |

---

## 3. MCP Server for Claude Integration

### Overview

Create an MCP (Model Context Protocol) server that exposes FIPA agent platform capabilities to Claude. This enables Claude to interact with the multi-agent system through natural conversation.

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       Claude Desktop                             │
│                            │                                     │
│                       MCP Client                                 │
└────────────────────────────┬────────────────────────────────────┘
                             │ JSON-RPC 2.0 (stdio)
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                    fipa-mcp Server (Rust)                        │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                   MCP Tools Exposed                         │ │
│  │  • fipa_create_agent     • fipa_send_message               │ │
│  │  • fipa_destroy_agent    • fipa_query_agent                │ │
│  │  • fipa_list_agents      • fipa_search_services            │ │
│  │  • fipa_register_service • fipa_get_messages               │ │
│  └────────────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                   MCP Resources                             │ │
│  │  • fipa://agents         • fipa://services                 │ │
│  │  • fipa://agents/{id}    • fipa://platform                 │ │
│  └────────────────────────────────────────────────────────────┘ │
│                            │                                     │
│                       gRPC Client                                │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                    FIPA Agent Platform                           │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐            │
│  │   AMS   │  │   DF    │  │ Agent A │  │ Agent B │   ...      │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘            │
└─────────────────────────────────────────────────────────────────┘
```

### MCP Tools

| Tool | Description | Parameters | Returns |
|------|-------------|------------|---------|
| `fipa_create_agent` | Create a new agent | `name`, `wasm_path`, `capabilities` | Agent ID |
| `fipa_destroy_agent` | Destroy an agent | `agent_id` | Success/failure |
| `fipa_list_agents` | List all agents | `filter` (optional) | Agent list |
| `fipa_query_agent` | Get agent details | `agent_id` | Agent info |
| `fipa_send_message` | Send ACL message | `sender`, `receiver`, `performative`, `content` | Message ID |
| `fipa_get_messages` | Get agent's mailbox | `agent_id`, `limit` | Message list |
| `fipa_search_services` | Search DF | `service_type`, `protocol` | Service list |
| `fipa_register_service` | Register service | `agent_id`, `service_desc` | Success/failure |
| `fipa_start_protocol` | Start interaction | `protocol`, `initiator`, `participants` | Conversation ID |

### MCP Resources

| Resource | URI Pattern | Description |
|----------|-------------|-------------|
| Agent list | `fipa://agents` | All registered agents |
| Agent detail | `fipa://agents/{id}` | Single agent details |
| Agent messages | `fipa://agents/{id}/messages` | Agent's message history |
| Services | `fipa://services` | DF service registry |
| Platform | `fipa://platform` | Platform information |

### Implementation

```rust
// src/bin/fipa-mcp.rs

use mcp_protocol_sdk::{
    Server, ServerBuilder, Tool, ToolHandler,
    Resource, ResourceHandler, JsonRpcError
};
use tonic::transport::Channel;
use crate::proto::agent_service_client::AgentServiceClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to FIPA platform
    let grpc_addr = std::env::var("FIPA_GRPC_ADDRESS")
        .unwrap_or_else(|_| "http://localhost:50051".to_string());

    let fipa_client = AgentServiceClient::connect(grpc_addr).await?;

    // Build MCP server
    let server = ServerBuilder::new("fipa-mcp", "1.0.0")
        .with_tool(CreateAgentTool::new(fipa_client.clone()))
        .with_tool(DestroyAgentTool::new(fipa_client.clone()))
        .with_tool(ListAgentsTool::new(fipa_client.clone()))
        .with_tool(SendMessageTool::new(fipa_client.clone()))
        .with_tool(SearchServicesTool::new(fipa_client.clone()))
        .with_resource(AgentListResource::new(fipa_client.clone()))
        .with_resource(PlatformResource::new(fipa_client.clone()))
        .build();

    // Run on stdio (for Claude Desktop)
    server.run_stdio().await?;

    Ok(())
}

/// Tool: Create a new agent
struct CreateAgentTool {
    client: AgentServiceClient<Channel>,
}

impl ToolHandler for CreateAgentTool {
    fn name(&self) -> &str { "fipa_create_agent" }

    fn description(&self) -> &str {
        "Create a new FIPA agent on the platform"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Unique name for the agent"
                },
                "wasm_path": {
                    "type": "string",
                    "description": "Path to WASM module (optional)"
                },
                "capabilities": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Agent capabilities"
                }
            },
            "required": ["name"]
        })
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, JsonRpcError> {
        let name = params["name"].as_str()
            .ok_or_else(|| JsonRpcError::invalid_params("name is required"))?;

        let request = CreateAgentRequest {
            name: name.to_string(),
            wasm_path: params["wasm_path"].as_str().map(String::from),
            capabilities: params["capabilities"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        };

        let response = self.client.clone()
            .create_agent(request)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(serde_json::json!({
            "agent_id": response.get_ref().agent_id,
            "status": "created"
        }))
    }
}

/// Tool: Send an ACL message
struct SendMessageTool {
    client: AgentServiceClient<Channel>,
}

impl ToolHandler for SendMessageTool {
    fn name(&self) -> &str { "fipa_send_message" }

    fn description(&self) -> &str {
        "Send a FIPA ACL message to an agent"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "sender": {
                    "type": "string",
                    "description": "Sender agent ID"
                },
                "receiver": {
                    "type": "string",
                    "description": "Receiver agent ID"
                },
                "performative": {
                    "type": "string",
                    "enum": ["REQUEST", "INFORM", "QUERY", "CFP", "PROPOSE", "ACCEPT", "REJECT"],
                    "description": "Message performative"
                },
                "content": {
                    "type": "string",
                    "description": "Message content"
                },
                "protocol": {
                    "type": "string",
                    "description": "Interaction protocol (optional)"
                }
            },
            "required": ["sender", "receiver", "performative", "content"]
        })
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, JsonRpcError> {
        let message = AclMessage {
            sender: params["sender"].as_str().unwrap().to_string(),
            receiver: params["receiver"].as_str().unwrap().to_string(),
            performative: params["performative"].as_str().unwrap().to_string(),
            content: params["content"].as_str().unwrap().as_bytes().to_vec(),
            protocol: params["protocol"].as_str().map(String::from),
            ..Default::default()
        };

        let response = self.client.clone()
            .send_message(message)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(serde_json::json!({
            "message_id": response.get_ref().message_id,
            "status": "sent"
        }))
    }
}

/// Resource: List of agents
struct AgentListResource {
    client: AgentServiceClient<Channel>,
}

impl ResourceHandler for AgentListResource {
    fn uri(&self) -> &str { "fipa://agents" }

    fn name(&self) -> &str { "FIPA Agents" }

    fn description(&self) -> &str {
        "List of all agents registered on the FIPA platform"
    }

    fn mime_type(&self) -> &str { "application/json" }

    async fn read(&self) -> Result<String, JsonRpcError> {
        let response = self.client.clone()
            .list_agents(ListAgentsRequest {})
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let agents: Vec<_> = response.get_ref().agents.iter()
            .map(|a| serde_json::json!({
                "id": a.id,
                "name": a.name,
                "status": a.status,
                "capabilities": a.capabilities
            }))
            .collect();

        Ok(serde_json::to_string_pretty(&agents)?)
    }
}
```

### Claude Desktop Configuration

Create `~/.config/claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "fipa": {
      "command": "fipa-mcp",
      "args": [],
      "env": {
        "FIPA_GRPC_ADDRESS": "http://localhost:50051"
      }
    }
  }
}
```

### Example Interactions with Claude

**Creating an Agent:**
```
User: Create a new agent called "weather-monitor" that can track weather data

Claude: I'll create a weather monitoring agent for you.

[Calls fipa_create_agent(name="weather-monitor", capabilities=["weather", "monitoring"])]

Done! I've created the agent "weather-monitor" with ID agent-abc123.
It's now running on the platform and ready to receive messages.
```

**Sending Messages:**
```
User: Ask the calculator agent to compute 42 * 17

Claude: I'll send a calculation request to the calculator agent.

[Calls fipa_send_message(
    sender="claude-assistant",
    receiver="calculator",
    performative="REQUEST",
    content="(calculate (* 42 17))"
)]

The calculator agent has processed the request. The result is 714.
```

**Querying Services:**
```
User: What services are available on the platform?

Claude: Let me check the directory facilitator.

[Reads resource fipa://services]

Here are the available services:
- **weather-service** (weather-agent): Provides weather data and forecasts
- **calculator-service** (calc-agent): Mathematical computations
- **translator-service** (translator-agent): Language translation
- **storage-service** (db-agent): Persistent data storage
```

**Starting a Protocol:**
```
User: Start a contract-net negotiation to find the best translator for French

Claude: I'll initiate a Contract Net Protocol to find translators.

[Calls fipa_start_protocol(
    protocol="CONTRACT_NET",
    initiator="claude-assistant",
    participants=["translator-1", "translator-2", "translator-3"]
)]

I've started the negotiation. Here are the proposals received:
- translator-1: 95% accuracy, $0.02/word
- translator-2: 98% accuracy, $0.05/word
- translator-3: 92% accuracy, $0.01/word

Would you like me to accept one of these proposals?
```

### File Structure

```
src/
├── mcp/
│   ├── mod.rs           # MCP module
│   ├── server.rs        # MCP server setup
│   ├── tools.rs         # Tool implementations
│   ├── resources.rs     # Resource implementations
│   └── prompts.rs       # Prompt templates (optional)
├── bin/
│   └── fipa-mcp.rs      # MCP server binary

Cargo.toml additions:
    [[bin]]
    name = "fipa-mcp"
    path = "src/bin/fipa-mcp.rs"
```

### Dependencies

```toml
[dependencies]
# MCP SDK (choose one)
mcp-protocol-sdk = "0.1"    # Community SDK
# OR
pmcp = "1.4"                # High-performance alternative

# JSON-RPC (if not included in MCP SDK)
jsonrpc-core = "18.0"
```

### Security Considerations

1. **Authentication**: MCP server should authenticate with FIPA platform
2. **Authorization**: Limit which agents Claude can control
3. **Rate Limiting**: Prevent excessive tool calls
4. **Audit Logging**: Log all Claude interactions
5. **Sandboxing**: Restrict agent creation to approved WASM modules

---

## Implementation Priority

| Enhancement | Effort | Impact | Dependencies |
|-------------|--------|--------|--------------|
| MCP Server | Medium | High | None |
| JSON-LD Codec | Low | Medium | json-ld crate |
| Schema Constraints | Low | Medium | None |
| OWL Integration | High | High | horned-owl |
| Local NLP (Rule-based) | Low | Medium | strsim |
| Local NLP (LLM) | High | High | candle |

### Recommended Order

1. **MCP Server** - Immediate value, enables Claude integration
2. **JSON-LD Codec** - Quick win for modern content format
3. **Schema Constraints** - Improves validation without major changes
4. **Local NLP (Rule-based)** - Simple translation without LLM overhead
5. **OWL Integration** - Full semantic web compatibility
6. **Local NLP (LLM)** - Advanced translation capabilities

---

## References

- [Model Context Protocol](https://modelcontextprotocol.io/)
- [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [Horned-OWL](https://github.com/phillord/horned-owl)
- [Candle ML Framework](https://github.com/huggingface/candle)
- [JSON-LD Specification](https://www.w3.org/TR/json-ld/)
- [FIPA ACL Specification](http://www.fipa.org/specs/fipa00061/)
