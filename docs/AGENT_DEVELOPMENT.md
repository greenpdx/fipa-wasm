# FIPA WASM Agent Development Guide

This guide explains how to develop WASM agents for the FIPA-WASM distributed agent system.

## Quick Start

### 1. Create a New Agent Project

```bash
# Install the generator (from the fipa-wasm-agents repo)
cargo install --path .

# Create a new agent
fipa-agent-new my-agent

# Or with a specific template
fipa-agent-new my-agent --template minimal
```

### 2. Build the Agent

```bash
cd my-agent

# Add WASM target (first time only)
rustup target add wasm32-wasip2

# Build
cargo build --release --target wasm32-wasip2
```

### 3. Deploy to a FIPA Node

```bash
fipa-cli deploy ./target/wasm32-wasip2/release/my_agent.wasm
```

## Project Templates

### Full Template (`--template full`)

Full-featured agent with access to all FIPA interfaces:

- **messaging** - Send/receive ACL messages
- **lifecycle** - Agent identity and state management
- **services** - Service registration and discovery
- **migration** - Node migration and cloning
- **storage** - Persistent key-value storage
- **logging** - Structured logging
- **timing** - Timers and scheduling
- **random** - Random number generation

Best for: Complex agents that need persistence, service discovery, or migration.

### Minimal Template (`--template minimal`)

Lightweight agent with basic interfaces:

- **messaging** - Send/receive ACL messages
- **lifecycle** - Agent identity and state
- **logging** - Basic logging

Best for: Simple agents that only need messaging.

### Stateless Template (`--template stateless`)

Pure message handler with no state:

- **messaging** - Message types only
- **logging** - Basic logging

Best for: Request-response services, message transformers.

## Agent Structure

### Required Exports

Every agent must export these functions:

```rust
// Full and Minimal templates
export init: func();              // Called once on startup
export run: func() -> bool;       // Called repeatedly, return false to stop
export shutdown: func();          // Called on shutdown

// Full template also exports
export handle-message: func(message: AclMessage) -> bool;

// Stateless template exports only
export handle-message: func(message: AclMessage) -> Option<AclMessage>;
```

### Basic Agent Pattern

```rust
wit_bindgen::generate!({
    world: "agent",
    path: "wit/fipa.wit",
});

use exports::fipa::agent::guest::Guest;
use fipa::agent::messaging::{self, AclMessage, Performative};
use fipa::agent::lifecycle;
use fipa::agent::logging::{self, LogLevel};

struct MyAgent {
    // Agent state
}

impl Guest for MyAgent {
    fn init() {
        // Initialize state, load from storage, register services
        let id = lifecycle::get_agent_id();
        logging::log(LogLevel::Info, &format!("Agent {} started", id.name));
    }

    fn run() -> bool {
        // Check for shutdown
        if lifecycle::is_shutdown_requested() {
            return false;
        }

        // Process messages
        while let Some(msg) = messaging::receive_message() {
            Self::handle_message(msg);
        }

        // Do other work...

        true // Keep running
    }

    fn shutdown() {
        // Save state, deregister services, cleanup
    }

    fn handle_message(msg: AclMessage) -> bool {
        // Process the message
        match msg.performative {
            Performative::Request => {
                // Handle request...
                true
            }
            _ => false
        }
    }
}

export!(MyAgent);
```

## FIPA ACL Messages

### Message Structure

```rust
struct AclMessage {
    message_id: String,           // Unique message ID
    performative: Performative,   // Message type (Request, Inform, etc.)
    sender: AgentId,              // Who sent it
    receivers: Vec<AgentId>,      // Who should receive it
    protocol: Option<ProtocolType>, // Interaction protocol
    conversation_id: Option<String>, // Conversation grouping
    in_reply_to: Option<String>,  // Reply to which message
    reply_by: Option<u64>,        // Deadline (Unix ms)
    language: Option<String>,     // Content language
    ontology: Option<String>,     // Ontology reference
    content: Vec<u8>,             // Binary content
}
```

### Performatives

FIPA defines 24 standard performatives:

| Performative | Use Case |
|--------------|----------|
| `Request` | Ask for an action |
| `Inform` | Share information |
| `QueryRef` | Ask for data |
| `QueryIf` | Ask yes/no question |
| `Cfp` | Call for proposals |
| `Propose` | Make a proposal |
| `AcceptProposal` | Accept a proposal |
| `RejectProposal` | Reject a proposal |
| `Agree` | Agree to do something |
| `Refuse` | Refuse to do something |
| `Failure` | Report failure |
| `Cancel` | Cancel a request |
| `Subscribe` | Subscribe to updates |
| ... | (see fipa.wit for full list) |

### Sending Messages

```rust
use fipa::agent::messaging::{self, AclMessage, Performative};
use fipa::agent::lifecycle;

fn send_request(target: &AgentId, content: &[u8]) {
    let msg = AclMessage {
        message_id: format!("msg-{}", uuid()),
        performative: Performative::Request,
        sender: lifecycle::get_agent_id(),
        receivers: vec![target.clone()],
        protocol: Some(ProtocolType::Request),
        conversation_id: Some(format!("conv-{}", uuid())),
        in_reply_to: None,
        reply_by: None,
        language: None,
        ontology: None,
        content: content.to_vec(),
    };

    match messaging::send_message(&msg) {
        Ok(id) => println!("Sent: {}", id),
        Err(e) => println!("Failed: {:?}", e),
    }
}
```

### Receiving Messages

```rust
// In run() or handle_message()
while let Some(msg) = messaging::receive_message() {
    match msg.performative {
        Performative::Request => handle_request(&msg),
        Performative::Inform => handle_inform(&msg),
        _ => {}
    }
}
```

## Persistent Storage

```rust
use fipa::agent::storage;

// Store data
storage::store("my_key", b"my_value")?;

// Load data
match storage::load("my_key") {
    Ok(data) => println!("Loaded: {:?}", data),
    Err(e) => println!("Not found or error: {:?}", e),
}

// Delete
storage::delete("my_key")?;

// Check existence
if storage::exists("my_key") { ... }

// List keys
let keys = storage::list_keys();
let prefixed = storage::list_keys_with_prefix("user:");

// Check quota
let used = storage::get_usage();
let quota = storage::get_quota();
```

## Service Registration

```rust
use fipa::agent::services::{self, ServiceDescription};

// Register a service
let service = ServiceDescription {
    name: "calculator".to_string(),
    description: "Math expression evaluator".to_string(),
    protocols: vec![ProtocolType::Query],
    ontology: Some("math".to_string()),
    properties: vec![
        ("operations".to_string(), "+,-,*,/".to_string()),
    ],
};

services::register_service(&service)?;

// Other agents can now find you
messaging::find_agents_by_service("calculator");

// Deregister when done
services::deregister_service("calculator")?;
```

## Timers and Scheduling

```rust
use fipa::agent::timing;

// Get current time
let now = timing::now();  // Unix milliseconds
let mono = timing::monotonic_now();  // Monotonic nanoseconds

// Schedule a timer (returns timer ID)
let timer_id = timing::schedule(5000);  // 5 seconds

// Schedule repeating timer
let repeat_id = timing::schedule_repeating(1000);  // Every second

// In run(), check for fired timers
for fired_id in timing::get_fired_timers() {
    if fired_id == timer_id {
        // Timer fired!
    }
}

// Cancel a timer
timing::cancel_timer(timer_id);
```

## Migration

```rust
use fipa::agent::migration::{self, MigrationReason};

// List available nodes
let nodes = migration::list_nodes();
for node in &nodes {
    println!("Node {}: load={}, agents={}",
             node.id, node.load, node.active_agents);
}

// Request migration
migration::migrate_to("node-2", MigrationReason::LoadBalancing)?;

// Clone to another node
let clone_id = migration::clone_to("node-3")?;

// Check if migrating
if migration::is_migrating() {
    // Migration in progress...
}
```

## Logging

```rust
use fipa::agent::logging::{self, LogLevel};

// Simple logging
logging::log(LogLevel::Info, "Agent started");
logging::log(LogLevel::Error, "Something went wrong");

// Structured logging with fields
logging::log_structured(
    LogLevel::Debug,
    "Processed message",
    &[
        ("message_id", msg.message_id.as_str()),
        ("sender", msg.sender.name.as_str()),
    ],
);

// Check if level is enabled (avoid expensive formatting)
if logging::is_enabled(LogLevel::Debug) {
    logging::log(LogLevel::Debug, &format!("Details: {:?}", data));
}
```

## Error Handling

The FIPA interfaces use `Result` types with specific error variants:

```rust
// Messaging errors
enum MessagingError {
    AgentNotFound(String),
    ProtocolNotAllowed(ProtocolType),
    NetworkError(String),
    Timeout,
    InvalidMessage(String),
    ConversationNotFound(String),
}

// Storage errors
enum StorageError {
    NotFound(String),
    QuotaExceeded,
    IoError(String),
    SerializationError(String),
    PermissionDenied,
}
```

## Examples

See the `examples/agents/` directory for complete working examples:

| Example | Description |
|---------|-------------|
| `ping-pong` | Basic messaging (request/inform) |
| `counter` | Persistent storage |
| `calculator` | Service registration, query protocol |

## Best Practices

1. **Handle shutdown gracefully** - Save state and deregister services
2. **Use conversations** - Group related messages with conversation_id
3. **Check performatives** - Different performatives have different semantics
4. **Log appropriately** - Use log levels to control verbosity
5. **Handle errors** - Don't panic, handle errors gracefully
6. **Respect quotas** - Storage has quotas, check before storing large data
7. **Use protocols** - Follow FIPA interaction protocols for interoperability

## Testing

```bash
# Build and run tests
cargo test

# Build WASM and test with runtime
cargo build --release --target wasm32-wasip2
fipa-cli test ./target/wasm32-wasip2/release/my_agent.wasm
```

## Resources

- [FIPA ACL Specification](http://www.fipa.org/specs/fipa00061/)
- [FIPA Interaction Protocols](http://www.fipa.org/repository/ips.php3)
- [WIT Bindgen Documentation](https://github.com/bytecodealliance/wit-bindgen)
- [WASI Preview 2](https://github.com/WebAssembly/WASI/tree/main/preview2)
