# FIPA WASM Mobile Agent System

A comprehensive implementation of FIPA (Foundation for Intelligent Physical Agents) protocols with mobile WASM agents that can migrate between distributed nodes.

## Overview

This system combines:
- **FIPA Protocols**: Complete implementation of 11+ FIPA interaction protocols
- **WASM Agents**: Portable, sandboxed agents compiled to WebAssembly
- **Mobile Code**: Agents can migrate between nodes with state preservation
- **Capability-Based Security**: Fine-grained permissions and resource limits
- **Distributed Architecture**: Multi-node network with agent directory services

## Architecture

### Core Components

1. **ACL Messages** (`acl_message.rs`)
   - Complete FIPA ACL message structures
   - Performatives, protocols, conversation management
   - Serialization support (JSON, Binary, FIPA String)

2. **Protocol State Machines** (`protocols.rs`)
   - Request, Query, Contract Net protocols
   - Subscribe, Propose, Auction protocols
   - Brokering and Recruiting protocols
   - Type-safe state transitions

3. **Agent Management** (`agent.rs`)
   - Mobile agent structures
   - State capture and restoration
   - Capability-based permissions
   - Conversation management

4. **Network Transport** (`network.rs`)
   - Inter-node communication
   - Agent directory services
   - Multiple codec support
   - Peer management

## FIPA Protocols Implemented

### Core Interaction Protocols
- **Request**: Simple request-response pattern
- **Query**: Information retrieval (query-if, query-ref)
- **Request-When**: Conditional execution

### Negotiation Protocols
- **Contract Net**: Task allocation through bidding
- **Iterated Contract Net**: Multi-round negotiation
- **Propose**: Proposal acceptance/rejection

### Auction Protocols
- **English Auction**: Ascending price auction
- **Dutch Auction**: Descending price auction

### Advanced Protocols
- **Brokering**: Broker-mediated interactions
- **Recruiting**: Recruiter-assisted agent discovery
- **Subscribe**: Continuous notifications

## Quick Start

### Docker (Recommended)

```bash
# Start single node
docker compose up -d

# View logs
docker compose logs -f

# Stop
docker compose down
```

### Build from Source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add WASM target
rustup target add wasm32-wasip1

# Install wasmtime (optional, for testing)
curl https://wasmtime.dev/install.sh -sSf | bash

# Build library
cargo build --release

# Run tests
cargo test
```

### Building Example Agents

```bash
# Build ping-pong agent
cd examples/agents/ping-pong
cargo build --release --target wasm32-wasip1

# Build counter agent
cd examples/agents/counter
cargo build --release --target wasm32-wasip1

# Build calculator agent
cd examples/agents/calculator
cargo build --release --target wasm32-wasip1
```

## Endpoints

| Port | Protocol | Description |
|------|----------|-------------|
| 9000 | gRPC | Agent messaging API |
| 9090 | HTTP | Prometheus metrics |

## gRPC API

### List Services

```bash
grpcurl -plaintext localhost:9000 list
```

### Health Check

```bash
grpcurl -plaintext -d '{"include_metrics": true}' \
  localhost:9000 fipa.v1.FipaAgentService/HealthCheck
```

### Get Node Info

```bash
grpcurl -plaintext -d '{}' \
  localhost:9000 fipa.v1.FipaAgentService/GetNodeInfo
```

### Send Message

```bash
grpcurl -plaintext -d '{
  "performative": "PERFORMATIVE_REQUEST",
  "sender": {"name": "client"},
  "receivers": [{"name": "target-agent"}],
  "content": "SGVsbG8=",
  "language": "text/plain"
}' localhost:9000 fipa.v1.FipaAgentService/SendMessage
```

### Find Agent

```bash
grpcurl -plaintext -d '{"agentId": {"name": "my-agent"}}' \
  localhost:9000 fipa.v1.FipaAgentService/FindAgent
```

### Find Service

```bash
grpcurl -plaintext -d '{"serviceName": "calculator", "maxResults": 10}' \
  localhost:9000 fipa.v1.FipaAgentService/FindService
```

### Available gRPC Methods

| Method | Description |
|--------|-------------|
| `HealthCheck` | Health status with optional metrics |
| `GetNodeInfo` | Node capabilities and info |
| `SendMessage` | Send ACL message to agent |
| `SubscribeMessages` | Stream messages (server streaming) |
| `FindAgent` | Locate an agent by name |
| `FindService` | Find agents providing a service |
| `MigrateAgent` | Migrate agent to this node |
| `CloneAgent` | Clone agent to this node |
| `GetWasmModule` | Request WASM module by hash |

## Metrics

Prometheus metrics available at `http://localhost:9090/metrics`:

```bash
curl localhost:9090/metrics
```

### Available Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `fipa_agents_spawned_total` | counter | Total agents spawned |
| `fipa_agents_active` | gauge | Current active agents |

Labels: `agent_type` (system, user)

## Usage Examples

### Creating an ACL Message

```rust,ignore
use fipa_wasm_agents::proto;

// Create agent IDs
let sender = proto::AgentId {
    name: "agent1".into(),
    ..Default::default()
};

let receiver = proto::AgentId {
    name: "agent2".into(),
    ..Default::default()
};

// Create an ACL message
let message = proto::AclMessage {
    message_id: uuid::Uuid::new_v4().to_string(),
    performative: proto::Performative::PerformativeRequest as i32,
    sender: Some(sender),
    receivers: vec![receiver],
    content: b"perform task X".to_vec(),
    protocol: proto::ProtocolType::ProtocolRequest as i32,
    conversation_id: "conv-123".into(),
    ..Default::default()
};
```

### Implementing a Protocol

```rust,ignore
use fipa_wasm_agents::protocol::{
    ProtocolStateMachine, RequestProtocol, ProtocolRole,
};
use fipa_wasm_agents::proto;

// Create a request protocol state machine
let mut protocol = RequestProtocol::new(ProtocolRole::Initiator);

// Create and process messages through the protocol
let request_msg = proto::AclMessage {
    performative: proto::Performative::PerformativeRequest as i32,
    // ... message fields
    ..Default::default()
};

// Process messages - state machine validates transitions
let response = protocol.process_message(&request_msg);
assert!(!protocol.is_complete());

// Continue with agree/inform messages...
```

### Creating a Mobile Agent

```rust,ignore
use fipa_wasm_agents::proto;
use fipa_wasm_agents::actor::AgentConfig;

// Define agent capabilities
let capabilities = proto::AgentCapabilities {
    max_memory_bytes: 64 * 1024 * 1024,
    max_cpu_time_ms: 5000,
    allowed_protocols: vec![
        proto::ProtocolType::ProtocolRequest as i32,
        proto::ProtocolType::ProtocolContractNet as i32,
    ],
    network_access: proto::NetworkAccessLevel::NetworkRestricted as i32,
    storage_quota_bytes: 10 * 1024 * 1024,
    can_migrate: true,
    can_spawn: false,
};

// Create agent configuration for spawning
let config = AgentConfig {
    id: proto::AgentId {
        name: "mobile-agent-1".into(),
        ..Default::default()
    },
    wasm_module: std::fs::read("agent.wasm").unwrap(),
    capabilities,
    initial_state: None,
    restart_strategy: Default::default(),
};

// Spawn the agent via supervisor actor
// supervisor.send(SpawnAgent { config }).await?;
```

### Agent Migration

```rust,ignore
use fipa_wasm_agents::proto;

// Create a migration package with agent state
let migration = proto::AgentMigration {
    agent_id: Some(proto::AgentId {
        name: "mobile-agent-1".into(),
        ..Default::default()
    }),
    wasm_module: Some(wasm_bytes),
    wasm_hash: compute_hash(&wasm_bytes),
    state: Some(captured_state),
    capabilities: Some(agent_capabilities),
    migration_history: vec!["node-1".into(), "node-2".into()],
    timestamp: chrono::Utc::now().timestamp_millis(),
    signature: None,
};

// Send via gRPC to target node
// client.migrate_agent(migration).await?;
```

## WIT Interface

WASM agents interact with the host through a WebAssembly Interface Types (WIT) interface:

```wit
// Send FIPA messages
send-message: func(
    receiver: agent-id,
    performative: performative,
    content: string,
    protocol: protocol-type
) -> result<message-id, error>;

// Receive messages
receive-message: func() -> result<option<message>, error>;

// Migrate to another node
migrate-to: func(node-id: string) -> result<_, string>;

// And more...
```

See `wit/fipa.wit` for the complete interface definition.

## Example Agents

Three example agents in `examples/agents/`:

### Ping-Pong
Simple echo agent - receives "ping", responds "pong".

### Counter
Persistent counter with commands: `inc`, `dec`, `get`, `reset`, `add:N`, `set:N`

### Calculator
Math expression evaluator with service registration: `+`, `-`, `*`, `/`, `%`, `^`, parentheses

## Docker Configuration

### Single Node (default)

```yaml
services:
  fipa-node:
    build: .
    ports:
      - "9000:9000"  # gRPC
      - "9090:9090"  # Metrics
    environment:
      - RUST_LOG=info
```

### Multi-Node Cluster

See commented section in `docker-compose.yml` for 3-node cluster setup with Raft consensus.

## Security Model

### Capability-Based Permissions

Agents declare capabilities that restrict their behavior:
- Resource limits (memory, CPU, storage)
- Protocol access
- Network access levels
- Migration permissions

### Code Signing

All agents must be cryptographically signed:
- SHA-256 hashing of bytecode and state
- Ed25519 signatures
- Public key infrastructure for trust chains

### Sandboxing

WASM provides strong isolation:
- Memory isolation
- Controlled host function access
- Resource limits enforced by runtime
- No direct system calls

## References

- [FIPA Specifications](http://www.fipa.org/repository/)
- [WebAssembly](https://webassembly.org/)
- [Wasmtime](https://wasmtime.dev/)
- [WIT (WebAssembly Interface Types)](https://github.com/WebAssembly/component-model)
