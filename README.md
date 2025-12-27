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

## Getting Started

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add WASM target
rustup target add wasm32-wasip1

# Install wasmtime (optional, for testing)
curl https://wasmtime.dev/install.sh -sSf | bash
```

### Building the Library

```bash
# Build library
cargo build --release

# Build with runtime support
cargo build --release --features runtime

# Run tests
cargo test
```

### Building Example Agents

```bash
# Build request handler agent
cargo build --release --target wasm32-wasip1 --example request_agent

# Build contractor agent
cargo build --release --target wasm32-wasip1 --example contractor_agent
```

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

## Integration with CR Monban

This system integrates with CR Monban for intelligent threat detection:

1. **Distributed Analysis**: Agents migrate to attack locations
2. **Collaborative Detection**: Contract Net for threat intelligence sharing
3. **Adaptive Response**: Dynamic deployment of countermeasures

## Development Roadmap

### Phase 1: Core WASM Runtime (Weeks 1-3)
- [x] WASM runtime setup
- [ ] Basic FIPA host functions
- [ ] State capture/restore
- [ ] Simple agent execution

### Phase 2: Protocol Integration (Weeks 4-6)
- [x] Protocol state machines
- [ ] Conversation manager
- [ ] Message routing
- [ ] Protocol validation

### Phase 3: Migration (Weeks 7-9)
- [ ] Agent serialization
- [ ] Migration protocol
- [ ] Signature verification
- [ ] State snapshots

### Phase 4: Network Layer (Weeks 10-12)
- [ ] Inter-node communication
- [ ] Agent directory
- [ ] Peer discovery
- [ ] Message forwarding

### Phase 5: Security (Weeks 13-15)
- [ ] Resource limiting
- [ ] Code signing
- [ ] Capability enforcement
- [ ] Audit logging

### Phase 6: Production (Weeks 16-18)
- [ ] Monitoring/metrics
- [ ] Fault tolerance
- [ ] Performance optimization
- [ ] Documentation

## Contributing

Contributions are welcome! Please see CONTRIBUTING.md for guidelines.

## License

This project is licensed under the MIT License - see LICENSE file for details.

## Contact

**Author**: SavageS
**GitHub**: github.com/greenpdx
**Project**: CR Monban Integration

## References

- [FIPA Specifications](http://www.fipa.org/repository/)
- [WebAssembly](https://webassembly.org/)
- [Wasmtime](https://wasmtime.dev/)
- [WIT (WebAssembly Interface Types)](https://github.com/WebAssembly/component-model)
