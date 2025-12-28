# RADE-like Features for FIPA WASM Agent System

This document describes the RADE (Rust Agent Development Framework) inspired features added to the FIPA WASM distributed agent system.

## Overview

The FIPA WASM agent system has been enhanced with features from RADE to provide:
- Rich behavior patterns for agent programming
- Formal platform agents (AMS and DF)
- Content language and ontology support
- Additional FIPA interaction protocols
- Comprehensive GUI and monitoring tools
- Security and access control
- Persistence and recovery
- Inter-platform communication

## Table of Contents

1. [Behavior System](#1-behavior-system)
2. [Platform Agents (AMS & DF)](#2-platform-agents-ams--df)
3. [Content & Ontology Framework](#3-content--ontology-framework)
4. [Additional Protocols](#4-additional-protocols)
5. [GUI & Monitoring Tools](#5-gui--monitoring-tools)
6. [Security](#6-security)
7. [Persistence & Recovery](#7-persistence--recovery)
8. [Inter-Platform Communication](#8-inter-platform-communication)

---

## 1. Behavior System

**Module**: `src/behavior/`

The behavior system provides RADE-style behavior patterns for structuring agent logic.

### Behavior Types

| Behavior | Description | Use Case |
|----------|-------------|----------|
| `OneShotBehaviour` | Executes once, then completes | Initialization, one-time tasks |
| `CyclicBehaviour` | Repeats indefinitely | Message polling, monitoring |
| `TickerBehaviour` | Executes at fixed intervals | Periodic updates, heartbeats |
| `WakerBehaviour` | Executes after a timeout | Delayed actions, reminders |
| `SequentialBehaviour` | Runs sub-behaviors in sequence | Multi-step workflows |
| `ParallelBehaviour` | Runs sub-behaviors concurrently | Concurrent operations |
| `FSMBehaviour` | Finite state machine with transitions | Complex protocols |

### Usage Example

```rust
use fipa_wasm_agents::behavior::{
    Behavior, BehaviorConfig, BehaviorType, BehaviorScheduler
};

// Create a ticker behavior that runs every 5 seconds
let config = BehaviorConfig {
    behavior_type: BehaviorType::Ticker,
    tick_interval: Some(Duration::from_secs(5)),
    ..Default::default()
};

let behavior = Behavior::new("heartbeat", config, |ctx| {
    println!("Heartbeat from agent: {}", ctx.agent_id);
    BehaviorResult::Continue
});

// Add to scheduler
scheduler.add(behavior);
```

### FSM Behavior

```rust
use fipa_wasm_agents::behavior::{FSMBehavior, StateHandler};

let mut fsm = FSMBehavior::new("protocol-handler");

fsm.register_state("INITIAL", |ctx| {
    // Handle initial state
    "WAITING".to_string()
});

fsm.register_state("WAITING", |ctx| {
    // Handle waiting state
    "COMPLETE".to_string()
});

fsm.set_initial_state("INITIAL");
fsm.add_terminal_state("COMPLETE");
```

---

## 2. Platform Agents (AMS & DF)

**Module**: `src/platform/`

### AMS (Agent Management System)

The AMS manages agent lifecycle and provides a platform-wide agent directory.

```rust
use fipa_wasm_agents::platform::{Ams, AmsConfig};

let config = AmsConfig {
    platform_name: "my-platform".to_string(),
    max_agents: 1000,
    enable_remote_management: true,
    ..Default::default()
};

let ams = Ams::new(config);

// Register an agent
ams.register_agent(agent_info).await?;

// Query agents
let agents = ams.search_agents(SearchFilter::by_state(AgentState::Active)).await;

// Suspend an agent
ams.suspend_agent("agent-id").await?;
```

**AMS Features**:
- Unique agent naming with collision detection
- Agent lifecycle management (create, suspend, resume, terminate)
- Platform access control
- Remote agent creation/destruction via ACL messages
- Agent state queries

### DF (Directory Facilitator)

The DF provides yellow pages service discovery.

```rust
use fipa_wasm_agents::platform::{Df, DfConfig, ServiceDescription};

let df = Df::new(DfConfig::default());

// Register a service
let service = ServiceDescription {
    name: "weather-service".to_string(),
    service_type: "weather".to_string(),
    protocols: vec!["fipa-request".to_string()],
    ontologies: vec!["weather-ontology".to_string()],
    properties: HashMap::new(),
};

df.register("weather-agent", service).await?;

// Search for services
let filter = SearchFilter::new()
    .with_type("weather")
    .with_protocol("fipa-request");

let results = df.search(filter).await;
```

**DF Features**:
- Service registration with rich descriptions
- Multi-criteria search (name, type, protocol, ontology, properties)
- Subscription to DF changes (notify on register/deregister)
- DF federation for multi-platform discovery
- Lease-based registrations with auto-expiry

---

## 3. Content & Ontology Framework

**Module**: `src/content/`

### Content Manager

The content manager handles encoding/decoding of message content.

```rust
use fipa_wasm_agents::content::{ContentManager, ContentElement, Concept};

let manager = ContentManager::new();

// Create a concept
let weather = Concept::new("WeatherReport")
    .with_slot("location", "Seattle")
    .with_slot("temperature", "72")
    .with_slot("conditions", "sunny");

// Encode to SL (Semantic Language)
let encoded = manager.encode(&weather.into(), "fipa-sl")?;

// Decode from SL
let decoded = manager.decode(&encoded, "fipa-sl")?;
```

### Codecs

| Codec | Language | Description |
|-------|----------|-------------|
| `SlCodec` | `fipa-sl` | FIPA Semantic Language (Lisp-like syntax) |
| `JsonCodec` | `application/json` | JSON encoding |

### SL Codec Examples

```rust
use fipa_wasm_agents::content::SlCodec;

let codec = SlCodec::new();

// Encode a concept
let sl = codec.encode(&concept)?;
// Result: (WeatherReport :location "Seattle" :temperature 72)

// Encode a predicate
let predicate = Predicate::new("is-raining")
    .with_arg(Term::String("Seattle".into()));
let sl = codec.encode(&predicate.into())?;
// Result: (is-raining "Seattle")

// Encode an action
let action = Action::new("get-weather")
    .with_actor("weather-agent")
    .with_arg(Term::String("Seattle".into()));
let sl = codec.encode(&action.into())?;
// Result: (action weather-agent (get-weather "Seattle"))
```

### Ontologies

```rust
use fipa_wasm_agents::content::{Ontology, OntologyRegistry, Schema, SchemaField};

// Define a custom ontology
let mut ontology = Ontology::new("weather-ontology");

ontology.add_concept_schema(Schema::new("WeatherReport")
    .with_field(SchemaField::required("location", SchemaType::String))
    .with_field(SchemaField::required("temperature", SchemaType::Integer))
    .with_field(SchemaField::optional("conditions", SchemaType::String))
);

// Register ontology
let mut registry = OntologyRegistry::new();
registry.register(ontology);

// Validate content against ontology
registry.validate("weather-ontology", &content)?;
```

**Built-in Ontologies**:
- `fipa-agent-management` - Agent lifecycle actions
- `fipa-ping` - Basic ping-pong testing

---

## 4. Additional Protocols

**Module**: `src/protocol/`

### Protocol Summary

| Protocol | Module | Description |
|----------|--------|-------------|
| Request | `request.rs` | Simple request-response |
| Query | `query.rs` | Information queries |
| Contract Net | `contract_net.rs` | Task delegation with bidding |
| Subscribe | `subscribe.rs` | Event subscription |
| **Propose** | `propose.rs` | Simple proposal acceptance/rejection |
| **English Auction** | `english_auction.rs` | Ascending price auction |
| **Dutch Auction** | `dutch_auction.rs` | Descending price auction |
| **Brokering** | `brokering.rs` | Broker-mediated interaction |
| **Recruiting** | `recruiting.rs` | Recruiter-assisted discovery |
| **Iterated Contract Net** | `iterated_contract_net.rs` | Multi-round negotiation |

### Propose Protocol

```rust
use fipa_wasm_agents::protocol::ProposeProtocol;

// As proposer
let mut protocol = ProposeProtocol::new_as_proposer();
protocol.send_propose("proposal-content")?;

// As responder
let mut protocol = ProposeProtocol::new_as_responder();
protocol.receive_propose(&message)?;
protocol.send_accept()?;  // or send_reject()
```

### English Auction

```rust
use fipa_wasm_agents::protocol::EnglishAuctionProtocol;

// As auctioneer
let mut auction = EnglishAuctionProtocol::new_as_auctioneer(
    100.0,  // starting price
    10.0,   // minimum increment
);

auction.start_auction()?;
auction.receive_bid("bidder-1", 110.0)?;
auction.receive_bid("bidder-2", 125.0)?;
auction.close_auction()?;

let winner = auction.winner();  // Some(("bidder-2", 125.0))
```

### Dutch Auction

```rust
use fipa_wasm_agents::protocol::DutchAuctionProtocol;

// As auctioneer
let mut auction = DutchAuctionProtocol::new_as_auctioneer(
    1000.0,  // starting price
    100.0,   // reserve price
    50.0,    // decrement amount
);

auction.start_auction()?;
auction.lower_price()?;  // Now 950
auction.lower_price()?;  // Now 900

// Bidder accepts current price
auction.accept_bid("buyer-1")?;
```

### Brokering Protocol

```rust
use fipa_wasm_agents::protocol::BrokeringProtocol;

// As broker
let mut broker = BrokeringProtocol::new_as_broker();
broker.receive_request(&client_request)?;

// Find suitable providers
broker.add_provider("provider-1");
broker.add_provider("provider-2");

// Forward request and collect responses
broker.forward_to_providers()?;
broker.receive_provider_response("provider-1", response)?;

// Consolidate and respond to client
broker.send_consolidated_response()?;
```

---

## 5. GUI & Monitoring Tools

**Module**: `src/tools/`

### Web Dashboard

**Endpoint**: `http://localhost:9091/dashboard`

```rust
use fipa_wasm_agents::tools::{Dashboard, DashboardConfig};

let config = DashboardConfig {
    bind_address: "0.0.0.0:9091".to_string(),
    refresh_interval_ms: 1000,
    enable_controls: true,
    ..Default::default()
};

let dashboard = Dashboard::new(config);
dashboard.start().await?;
```

**Features**:
- Real-time agent listing and status
- Agent creation/destruction controls
- Platform topology visualization
- Message inspection and filtering
- Metrics visualization
- Container management

### CLI Tools

**Binary**: `fipa-cli`

```bash
# Agent management
fipa-cli agents list
fipa-cli agents create my-agent --wasm path/to/agent.wasm
fipa-cli agents destroy my-agent
fipa-cli agents status my-agent

# Service discovery
fipa-cli services list
fipa-cli services search weather
fipa-cli services register my-service --type weather

# Messaging
fipa-cli messages send '{"performative": "request", ...}'
fipa-cli messages sniff my-agent

# Cluster management
fipa-cli nodes list
fipa-cli nodes info node-1
```

### TUI (Terminal UI)

**Binary**: `fipa-tui`

```bash
fipa-tui --grpc-address localhost:50051
```

**Features**:
- Interactive agent browser with vim-style navigation
- Real-time message stream view
- Agent creation/destruction dialogs
- Service discovery panel
- Metrics dashboard with charts
- Keyboard shortcuts (j/k navigation, Enter to select, q to quit)

### Sniffer Agent

```rust
use fipa_wasm_agents::tools::{Sniffer, SnifferConfig, TraceFilter};

let config = SnifferConfig {
    max_messages: 10000,
    enable_persistence: true,
    ..Default::default()
};

let sniffer = Sniffer::new(config);

// Start sniffing specific agents
sniffer.sniff_agent("agent-1").await;
sniffer.sniff_agent("agent-2").await;

// Apply filters
sniffer.set_filter(TraceFilter::new()
    .with_performative(Performative::Request)
    .with_protocol("fipa-request")
);

// Get trace
let trace = sniffer.get_trace().await;

// Export
let csv = trace.to_csv();
let json = trace.to_json();
```

---

## 6. Security

**Module**: `src/security/`

### Authentication

```rust
use fipa_wasm_agents::security::{
    Authenticator, AgentCredentials, Certificate, Token
};

let authenticator = Authenticator::new();

// Configure API keys
authenticator.add_api_key("secret-key", vec!["admin"]);

// Authenticate with token
let credentials = AgentCredentials::Token(Token::bearer("secret-key"));
let session = authenticator.authenticate(&credentials).await?;

// Check session
assert!(session.is_authenticated());
assert!(session.has_role("admin"));
```

### Credentials Types

| Type | Description |
|------|-------------|
| `Anonymous` | No authentication |
| `Certificate` | X.509-style certificate with public key |
| `Token` | Bearer token, API key, or JWT |

### Permissions

```rust
use fipa_wasm_agents::security::{
    Permission, PermissionSet, Resource, Action
};

let mut permissions = PermissionSet::new();

// Grant permissions
permissions.grant(Permission::new(
    Resource::agent("my-agent"),
    Action::Read,
));

permissions.grant(Permission::new(
    Resource::service("*"),  // Wildcard
    Action::All,
));

// Check permissions
let allowed = permissions.check(
    &Resource::agent("my-agent"),
    &Action::Read,
);
```

### RBAC Policy

```rust
use fipa_wasm_agents::security::{PolicyEngine, Role, RoleBinding};

let mut engine = PolicyEngine::new();

// Define roles
let admin_role = Role::new("admin")
    .with_permission(Permission::new(Resource::all(), Action::All));

let viewer_role = Role::new("viewer")
    .with_permission(Permission::new(Resource::all(), Action::Read));

engine.add_role(admin_role);
engine.add_role(viewer_role);

// Bind roles to principals
engine.add_binding(RoleBinding::new("user@example.com", "viewer"));

// Check authorization
let allowed = engine.authorize("user@example.com", &resource, &action).await?;
```

### Security Manager

```rust
use fipa_wasm_agents::security::{SecurityManager, SecurityConfig};

let config = SecurityConfig {
    enable_authentication: true,
    enable_authorization: true,
    token_expiry_secs: 3600,
    ..Default::default()
};

let manager = SecurityManager::new(config);

// Full authentication + authorization flow
let session = manager.authenticate(&credentials).await?;
manager.authorize(&session, &resource, &action).await?;
```

---

## 7. Persistence & Recovery

**Module**: `src/persistence/`

### Persistence Manager

```rust
use fipa_wasm_agents::persistence::{
    PersistenceManager, PersistenceConfig, FileStorage
};

let config = PersistenceConfig {
    enabled: true,
    storage_path: "/var/lib/fipa/snapshots".into(),
    snapshot_interval_secs: 300,  // Every 5 minutes
    max_snapshots: 10,
    ..Default::default()
};

let storage = FileStorage::new(&config.storage_path)?;
let manager = PersistenceManager::new(config, Box::new(storage));

// Start background snapshots
manager.start().await;

// Manual snapshot
manager.snapshot_agent(&agent_id, &agent_state).await?;
manager.snapshot_platform(&platform_state).await?;
```

### Snapshot Types

| Type | Description |
|------|-------------|
| `AgentSnapshot` | Agent state, behaviors, conversations |
| `PlatformSnapshot` | Platform configuration, agent directory |
| `ServiceSnapshot` | DF service registrations |
| `ConversationSnapshot` | In-progress conversation state |

### Recovery Engine

```rust
use fipa_wasm_agents::persistence::{RecoveryEngine, RecoveryState};

let engine = RecoveryEngine::new(storage);

// Load recovery state
let state = engine.load_recovery_state().await?;

// Recover agents
for agent in state.agents {
    if agent.should_restart() {
        // Restart agent with saved state
        supervisor.spawn_with_state(agent.config, agent.state).await?;
    }
}

// Resume conversations
for conversation in state.conversations {
    if !conversation.is_terminal() {
        // Resume conversation
        protocol.restore_state(conversation.state)?;
    }
}
```

### Storage Backends

| Backend | Description |
|---------|-------------|
| `FileStorage` | JSON files on disk |
| `MemoryStorage` | In-memory (for testing) |

---

## 8. Inter-Platform Communication

**Module**: `src/interplatform/`

### Architecture

```
+------------------+     +------------------+     +------------------+
|  Local Agent     |---->|       ACC        |---->|  Remote Platform |
+------------------+     +------------------+     +------------------+
                                 |
                                 v
                         +------------------+
                         |  MTP Registry    |
                         +------------------+
                         | - HTTP MTP       |
                         | - gRPC MTP       |
                         | - Custom MTPs    |
                         +------------------+
```

### Message Transport Protocol (MTP)

```rust
use fipa_wasm_agents::interplatform::{
    Mtp, MtpRegistry, MtpConfig, HttpMtp
};

// Create MTP registry
let mut registry = MtpRegistry::new();

// Register HTTP MTP
let http_mtp = HttpMtp::new(MtpConfig::default());
registry.register("http", Box::new(http_mtp));
registry.set_default("http");

// Activate MTPs
registry.activate_all(&config).await?;
```

### Agent Communication Channel (ACC)

```rust
use fipa_wasm_agents::interplatform::{Acc, AccConfig, MessageEnvelope};

let config = AccConfig {
    platform_name: "my-platform".to_string(),
    enable_buffering: true,
    max_retries: 3,
    retry_delay_ms: 1000,
    ..Default::default()
};

let acc = Acc::new(config)
    .with_mtp_registry(registry)
    .with_local_delivery(|envelope| {
        // Handle local message delivery
        deliver_to_agent(envelope);
    });

acc.start().await?;

// Send a message
let envelope = MessageEnvelope::new("sender-agent", message_bytes)
    .to("receiver@remote-platform.example.com:8080")
    .with_protocol("fipa-request")
    .with_conversation_id("conv-123");

let result = acc.send(envelope).await?;
```

### Address Resolution

```rust
use fipa_wasm_agents::interplatform::{
    AgentAddress, PlatformAddress, AddressResolver
};

// Parse addresses
let local = AgentAddress::parse("my-agent")?;           // Local agent
let remote = AgentAddress::parse("agent@platform")?;    // Remote by platform
let url = AgentAddress::parse("agent@http://host:8080")?;  // Remote by URL

// Platform registration
let resolver = AddressResolver::new("local-platform");

let remote_platform = PlatformAddress::new("remote-platform")
    .with_http("http://remote.example.com:8080")
    .with_grpc("grpc://remote.example.com:9090");

resolver.register_platform(remote_platform).await;

// Resolve address to transport endpoint
let endpoints = resolver.resolve(&remote_address).await?;
```

### Message Envelope

```rust
use fipa_wasm_agents::interplatform::{MessageEnvelope, TransportInfo};

let envelope = MessageEnvelope::new("sender", payload)
    .to("receiver@remote-platform")
    .cc("observer@another-platform")
    .with_protocol("fipa-request")
    .with_conversation_id("conv-123")
    .with_language("fipa-sl")
    .with_ontology("my-ontology")
    .with_reply_to("sender@my-platform");

// Serialize for transport (FIPA-compliant XML)
let xml = envelope.to_xml();

// Transport info
let info = TransportInfo {
    mtp_name: "http".to_string(),
    sent_at: Utc::now(),
    hops: vec![],
};
```

### HTTP MTP

```rust
use fipa_wasm_agents::interplatform::HttpMtp;

let mtp = HttpMtp::new(MtpConfig {
    timeout_secs: 30,
    max_message_size: 1024 * 1024,  // 1MB
    ..Default::default()
});

// Send envelope via HTTP POST
let result = mtp.send(&envelope).await?;

// Receive incoming messages
while let Some(envelope) = mtp.receive().await {
    acc.receive(envelope).await?;
}
```

---

## Configuration

### Complete Example

```rust
use fipa_wasm_agents::{
    behavior::BehaviorScheduler,
    platform::{Ams, Df},
    security::SecurityManager,
    persistence::PersistenceManager,
    interplatform::Acc,
    tools::Dashboard,
};

// Initialize platform components
let ams = Ams::new(AmsConfig::default());
let df = Df::new(DfConfig::default());
let security = SecurityManager::new(SecurityConfig::default());
let persistence = PersistenceManager::new(PersistenceConfig::default(), storage);
let acc = Acc::new(AccConfig::default());
let dashboard = Dashboard::new(DashboardConfig::default());

// Start all services
ams.start().await?;
df.start().await?;
acc.start().await?;
persistence.start().await;
dashboard.start().await?;
```

---

## Testing

All features include comprehensive test coverage:

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test behavior::
cargo test platform::
cargo test content::
cargo test protocol::
cargo test security::
cargo test persistence::
cargo test interplatform::
cargo test tools::
```

**Test counts by module**:
- Behavior: 4 tests
- Platform (AMS/DF): 5 tests
- Content/Ontology: 17 tests
- Protocols: 18 tests
- Security: 24 tests
- Persistence: 22 tests
- Inter-platform: 24 tests
- Tools: 8 tests

**Total**: 174 tests

---

## Dependencies Added

```toml
[dependencies]
# HTTP client for inter-platform communication
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }

# TUI framework
ratatui = "0.29"
crossterm = "0.28"

# Date/time handling
chrono = { version = "0.4", features = ["serde"] }
```

---

## Migration from Previous Versions

If upgrading from a version without RADE features:

1. **Behaviors**: Wrap existing agent logic in behavior types for better structure
2. **AMS/DF**: Use formal AMS and DF instead of raw ActorRegistry
3. **Content**: Use ContentManager for message encoding instead of raw bytes
4. **Security**: Enable SecurityManager for authentication/authorization
5. **Persistence**: Configure PersistenceManager for crash recovery
6. **Inter-platform**: Use ACC for cross-platform messaging

---

## References

- [FIPA Specifications](http://www.fipa.org/specifications/)
- [JADE Documentation](https://jade.tilab.com/documentation/)
- [FIPA ACL Message Structure](http://www.fipa.org/specs/fipa00061/)
- [FIPA Interaction Protocols](http://www.fipa.org/specs/fipa00025/)
