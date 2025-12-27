# FIPA-WASM Distributed Agent System Architecture

**Version:** 0.2.0
**Last Updated:** December 2024
**Author:** SavageS

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [System Overview](#system-overview)
3. [Architecture Decisions](#architecture-decisions)
4. [Module Structure](#module-structure)
5. [Core Components](#core-components)
6. [Network Architecture](#network-architecture)
7. [WASM Runtime](#wasm-runtime)
8. [Distributed Consensus](#distributed-consensus)
9. [Security Model](#security-model)
10. [Message Formats](#message-formats)
11. [API Reference](#api-reference)
12. [Deployment](#deployment)

---

## Executive Summary

The FIPA-WASM Distributed Agent System is a next-generation implementation of FIPA (Foundation for Intelligent Physical Agents) protocols combined with WebAssembly-based mobile agents. The system enables autonomous software agents to:

- **Communicate** using standardized FIPA ACL (Agent Communication Language) protocols
- **Migrate** between distributed nodes while preserving state
- **Coordinate** using Raft consensus for reliable distributed operations
- **Discover** each other via peer-to-peer networking with automatic service discovery

### Key Technologies

| Component | Technology | Purpose |
|-----------|------------|---------|
| Concurrency | **Actix** | Actor model for message passing and supervision |
| Networking | **libp2p** | Peer discovery, NAT traversal, transport multiplexing |
| RPC | **gRPC/tonic** | Typed inter-service communication |
| Serialization | **Protocol Buffers** | Efficient binary message encoding |
| WASM Runtime | **Wasmtime** | Component model with WASI Preview 2 |
| Consensus | **openraft** | Raft-based distributed state machine |
| Storage | **RocksDB** | Persistent storage for Raft logs and agent state |

---

## System Overview

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           FIPA-WASM Node                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐             │
│  │   Agent Actor   │  │   Agent Actor   │  │   Agent Actor   │  ...        │
│  │  ┌───────────┐  │  │  ┌───────────┐  │  │  ┌───────────┐  │             │
│  │  │WASM Module│  │  │  │WASM Module│  │  │  │WASM Module│  │             │
│  │  └───────────┘  │  │  └───────────┘  │  │  └───────────┘  │             │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘             │
│           │                    │                    │                       │
│  ┌────────┴────────────────────┴────────────────────┴────────┐             │
│  │                      Supervisor Tree                       │             │
│  └────────────────────────────┬──────────────────────────────┘             │
│                               │                                             │
│  ┌────────────────────────────┼──────────────────────────────┐             │
│  │                    Actor Registry                          │             │
│  └────────────────────────────┬──────────────────────────────┘             │
│                               │                                             │
├───────────────────────────────┼─────────────────────────────────────────────┤
│  ┌────────────────┐  ┌────────┴───────┐  ┌─────────────────┐               │
│  │ Agent Directory│  │ Service Registry│  │ Raft Consensus │               │
│  │   (Replicated) │  │   (Replicated)  │  │                 │               │
│  └────────────────┘  └────────────────┘  └─────────────────┘               │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                        Network Layer                                 │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐            │   │
│  │  │  libp2p  │  │  gRPC    │  │  mDNS    │  │ Kademlia │            │   │
│  │  │ Transport│  │ Services │  │ Discovery│  │   DHT    │            │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────┘            │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                          ┌─────────┴─────────┐
                          │    Other Nodes    │
                          └───────────────────┘
```

### Data Flow

1. **Incoming Message**: libp2p → gRPC Service → Actor Registry → Agent Actor → WASM Module
2. **Outgoing Message**: WASM Module → Host Functions → Actor → Network Layer → libp2p
3. **Migration**: Agent Actor → Capture State → Serialize → Network → Target Node → Restore

---

## Architecture Decisions

### ADR-001: Actor Model with Actix

**Decision:** Use Actix actor framework for concurrency and supervision.

**Rationale:**
- Mature ecosystem with extensive documentation
- Built-in supervision trees for fault tolerance
- Async-first design integrates well with Tokio
- Message-passing model aligns with FIPA agent semantics

**Alternatives Considered:**
- Bastion: Erlang-inspired but less mature
- Raw Tokio tasks: No built-in supervision

### ADR-002: Hybrid libp2p + gRPC Networking

**Decision:** Use libp2p for peer discovery and transport, gRPC for typed RPC.

**Rationale:**
- libp2p provides peer discovery (mDNS, Kademlia), NAT traversal, and transport encryption
- gRPC provides strongly-typed service interfaces and code generation
- Combined approach leverages strengths of both

**Implementation:**
- libp2p handles node discovery and connection management
- gRPC services run over libp2p streams using request-response protocol
- Protobuf used for both gRPC and libp2p message encoding

### ADR-003: WASI Preview 2 Component Model

**Decision:** Use WASI Preview 2 with the Component Model for WASM agents.

**Rationale:**
- Typed interfaces via WIT (WebAssembly Interface Types)
- Better composability than Preview 1
- Resource types for handles and capabilities
- Future-proof as WASI stabilizes

### ADR-004: Raft Consensus for Distributed State

**Decision:** Use openraft for consensus on critical shared state.

**Rationale:**
- Agent directory requires consistent view across nodes
- Service registry needs linearizable operations
- Leader election simplifies coordination protocols

**What's Replicated:**
- Agent location mapping (agent ID → node ID)
- Service registry (service name → provider agents)
- Global configuration

**What's NOT Replicated:**
- Individual agent state (migrates with agent)
- Conversation state (local to agent)
- Transient messages

---

## Module Structure

```
src/
├── lib.rs                      # Public API, re-exports
│
├── actor/                      # Actor Framework Integration
│   ├── mod.rs                  # Module exports
│   ├── agent_actor.rs          # WASM agent wrapper actor
│   ├── supervisor.rs           # Supervision tree management
│   ├── registry.rs             # Actor name/address registry
│   └── messages.rs             # Inter-actor message types
│
├── protocol/                   # FIPA Protocol Implementations
│   ├── mod.rs                  # Protocol trait, exports
│   ├── state_machine.rs        # Generic state machine trait
│   ├── request.rs              # FIPA Request protocol
│   ├── query.rs                # FIPA Query protocol
│   ├── contract_net.rs         # FIPA Contract Net protocol
│   ├── subscribe.rs            # FIPA Subscribe protocol
│   ├── auction.rs              # English/Dutch auction protocols
│   └── brokering.rs            # Brokering/Recruiting protocols
│
├── network/                    # Network Layer
│   ├── mod.rs                  # Network subsystem entry
│   ├── transport.rs            # libp2p swarm configuration
│   ├── discovery.rs            # mDNS + Kademlia discovery
│   ├── grpc.rs                 # tonic gRPC service impls
│   ├── routing.rs              # Message routing actor
│   └── nat.rs                  # NAT traversal (AutoNAT, relay)
│
├── wasm/                       # WASM Runtime
│   ├── mod.rs                  # Runtime exports
│   ├── runtime.rs              # Wasmtime component runtime
│   ├── bindings.rs             # WIT-generated bindings
│   ├── host.rs                 # Host function implementations
│   └── state.rs                # State capture/restore
│
├── consensus/                  # Distributed Consensus
│   ├── mod.rs                  # Consensus exports
│   ├── raft.rs                 # openraft integration
│   ├── state.rs                # Replicated state machine
│   └── storage.rs              # RocksDB log storage
│
├── registry/                   # Service Discovery
│   ├── mod.rs                  # Registry exports
│   ├── agent_directory.rs      # Agent location service
│   ├── service_registry.rs     # Service discovery
│   └── health.rs               # Health checking
│
├── proto/                      # Protocol Buffers
│   ├── fipa.proto              # Message definitions
│   └── mod.rs                  # Generated code re-exports
│
└── bin/                        # Binaries
    ├── agent_node.rs           # Main node binary
    └── cli.rs                  # CLI management tool
```

---

## Core Components

### 1. Agent Actor

The `AgentActor` wraps a WASM agent and manages its lifecycle:

```rust
pub struct AgentActor {
    /// Agent identifier
    agent_id: AgentId,

    /// WASM runtime instance
    runtime: WasmRuntime,

    /// Active protocol conversations
    conversations: HashMap<String, Box<dyn ProtocolStateMachine>>,

    /// Incoming message queue
    mailbox: VecDeque<AclMessage>,

    /// Agent capabilities/permissions
    capabilities: AgentCapabilities,

    /// Reference to supervisor
    supervisor: Addr<Supervisor>,

    /// Reference to network actor
    network: Addr<NetworkActor>,
}
```

**Actor Messages:**

| Message | Description |
|---------|-------------|
| `DeliverMessage` | Deliver an ACL message to the agent |
| `CaptureState` | Capture agent state for migration |
| `MigrateTo` | Initiate migration to another node |
| `Shutdown` | Graceful shutdown request |

### 2. Supervisor

Manages agent lifecycle with restart strategies:

```rust
pub struct Supervisor {
    /// Supervised agent actors
    agents: HashMap<AgentId, Addr<AgentActor>>,

    /// Restart strategy per agent
    strategies: HashMap<AgentId, RestartStrategy>,

    /// Failure counts for backoff
    failure_counts: HashMap<AgentId, FailureRecord>,
}

pub enum RestartStrategy {
    /// Always restart immediately
    Immediate,

    /// Restart with exponential backoff
    Backoff { initial_ms: u64, max_ms: u64, multiplier: f64 },

    /// Stop after N failures in time window
    MaxFailures { count: u32, window_secs: u64 },

    /// Never restart
    None,
}
```

### 3. Protocol State Machines

Each FIPA protocol is implemented as a state machine:

```rust
pub trait ProtocolStateMachine: Send + Sync {
    /// Get protocol type identifier
    fn protocol_type(&self) -> ProtocolType;

    /// Validate an incoming message
    fn validate(&self, msg: &AclMessage) -> Result<(), ProtocolError>;

    /// Process message and transition state
    fn process(&mut self, msg: AclMessage) -> Result<ProcessResult, ProtocolError>;

    /// Check if protocol is in terminal state
    fn is_complete(&self) -> bool;

    /// Serialize state for migration
    fn serialize_state(&self) -> Vec<u8>;

    /// Restore state after migration
    fn restore_state(data: &[u8]) -> Result<Self, ProtocolError> where Self: Sized;
}

pub enum ProcessResult {
    /// Continue waiting for messages
    Continue,

    /// Send response message
    Respond(AclMessage),

    /// Protocol completed successfully
    Complete(CompletionData),

    /// Protocol failed
    Failed(String),
}
```

---

## Network Architecture

### libp2p Configuration

```rust
pub struct NetworkConfig {
    /// Listen addresses
    pub listen_addrs: Vec<Multiaddr>,

    /// Bootstrap peers for DHT
    pub bootstrap_peers: Vec<(PeerId, Multiaddr)>,

    /// Enable mDNS for local discovery
    pub enable_mdns: bool,

    /// Enable Kademlia DHT
    pub enable_kademlia: bool,

    /// Enable relay for NAT traversal
    pub enable_relay: bool,

    /// Node identity keypair
    pub keypair: Keypair,
}
```

**Protocols Used:**

| Protocol | Purpose |
|----------|---------|
| Noise | Encryption and authentication |
| Yamux | Stream multiplexing |
| mDNS | Local network peer discovery |
| Kademlia | Distributed hash table for wide-area discovery |
| GossipSub | Pub/sub for broadcast messages |
| Request-Response | Direct agent-to-agent messages |
| AutoNAT | NAT type detection |
| Relay | Fallback for NAT traversal |
| DCUtR | Direct Connection Upgrade through Relay |

### Message Flow

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Agent A     │     │  Network     │     │  Agent B     │
│  (Node 1)    │     │  Layer       │     │  (Node 2)    │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │                    │                    │
       │ send_message()     │                    │
       ├───────────────────>│                    │
       │                    │ libp2p route       │
       │                    ├───────────────────>│
       │                    │                    │ DeliverMessage
       │                    │                    ├─────────────>
       │                    │                    │ process in WASM
       │                    │<───────────────────┤ response
       │                    │                    │
       │<───────────────────┤                    │
       │ receive_message()  │                    │
       │                    │                    │
```

---

## WASM Runtime

### WIT Interface (WASI Preview 2)

```wit
package fipa:agent@0.2.0;

interface messaging {
    enum performative {
        request, inform, query-if, query-ref, cfp, propose,
        accept-proposal, reject-proposal, agree, refuse, failure,
        inform-done, inform-result, not-understood, subscribe, cancel,
    }

    enum protocol-type {
        request, query, request-when, contract-net,
        iterated-contract-net, propose, brokering, recruiting,
        subscribe, english-auction, dutch-auction,
    }

    record agent-id {
        name: string,
        addresses: list<string>,
    }

    record acl-message {
        message-id: string,
        performative: performative,
        sender: agent-id,
        receivers: list<agent-id>,
        protocol: option<protocol-type>,
        conversation-id: option<string>,
        content: list<u8>,
    }

    send-message: func(message: acl-message) -> result<string, string>;
    receive-message: func() -> option<acl-message>;
    find-agents-by-service: func(service: string) -> result<list<agent-id>, string>;
}

interface migration {
    record node-info {
        id: string,
        load: f32,
        latency-ms: u32,
        available-memory: u64,
    }

    get-current-node: func() -> string;
    list-nodes: func() -> list<node-info>;
    migrate-to: func(node-id: string) -> result<_, string>;
}

interface storage {
    store: func(key: string, value: list<u8>) -> result<_, string>;
    load: func(key: string) -> result<list<u8>, string>;
    delete: func(key: string) -> result<_, string>;
}

interface logging {
    enum log-level { trace, debug, info, warn, error }
    log: func(level: log-level, message: string);
}

world agent {
    import wasi:io/poll@0.2.0;
    import wasi:clocks/monotonic-clock@0.2.0;
    import messaging;
    import migration;
    import storage;
    import logging;

    export init: func();
    export run: func() -> bool;
    export shutdown: func();
    export handle-message: func(message: messaging.acl-message) -> bool;
}
```

### State Capture for Migration

```rust
pub struct AgentSnapshot {
    /// Agent identifier
    pub agent_id: AgentId,

    /// WASM module bytecode
    pub wasm_module: Vec<u8>,

    /// Linear memory snapshot
    pub memory: Vec<u8>,

    /// Global variable values
    pub globals: Vec<GlobalValue>,

    /// Active conversation states
    pub conversations: Vec<ConversationSnapshot>,

    /// Persistent storage data
    pub storage: HashMap<String, Vec<u8>>,

    /// Migration history
    pub migration_history: Vec<NodeId>,

    /// Cryptographic signature
    pub signature: Option<Vec<u8>>,
}
```

---

## Distributed Consensus

### Raft Integration

The system uses openraft for consensus on shared state:

```rust
pub type NodeId = u64;

pub struct FipaRaft {
    raft: Raft<TypeConfig>,
    network: Arc<RaftNetwork>,
    storage: Arc<RaftStorage>,
}

/// State machine for replicated data
pub struct StateMachine {
    /// Agent directory: agent_id -> node_id
    agent_directory: HashMap<String, NodeId>,

    /// Service registry: service_name -> Vec<(agent_id, metadata)>
    service_registry: HashMap<String, Vec<ServiceEntry>>,

    /// Last applied log index
    last_applied: LogId,
}

/// Commands that can be applied to state machine
pub enum Command {
    RegisterAgent { agent_id: String, node_id: NodeId },
    DeregisterAgent { agent_id: String },
    RegisterService { service: ServiceEntry },
    DeregisterService { service_name: String, agent_id: String },
    MigrateAgent { agent_id: String, from: NodeId, to: NodeId },
}
```

### Consistency Guarantees

| Operation | Consistency | Notes |
|-----------|-------------|-------|
| Find agent location | Linearizable | Via Raft read |
| Register agent | Linearizable | Via Raft write |
| Send message | At-most-once | Direct delivery |
| Migration | Exactly-once | Two-phase with Raft |

---

## Security Model

### Capability-Based Permissions

Each agent declares capabilities that restrict its behavior:

```rust
pub struct AgentCapabilities {
    /// Maximum memory in bytes
    pub max_memory: u64,

    /// Maximum execution time per call
    pub max_execution_time: Duration,

    /// Allowed protocols
    pub allowed_protocols: HashSet<ProtocolType>,

    /// Network access level
    pub network_access: NetworkAccess,

    /// Storage quota in bytes
    pub storage_quota: u64,

    /// Can this agent migrate?
    pub migration_allowed: bool,

    /// Can this agent spawn sub-agents?
    pub spawn_allowed: bool,
}

pub enum NetworkAccess {
    /// No network access
    None,

    /// Only local node agents
    LocalOnly,

    /// Specific nodes/agents allowed
    Restricted(Vec<String>),

    /// Full network access
    Unrestricted,
}
```

### Code Signing

All agent packages are cryptographically signed:

```rust
pub struct AgentPackage {
    /// The agent data
    pub agent: AgentSnapshot,

    /// SHA-256 hash of agent data
    pub hash: [u8; 32],

    /// Ed25519 signature of hash
    pub signature: [u8; 64],

    /// Signer's public key
    pub public_key: [u8; 32],

    /// Signature timestamp
    pub timestamp: u64,
}

impl AgentPackage {
    pub fn verify(&self) -> Result<(), SignatureError> {
        // 1. Recompute hash
        let computed_hash = sha256(&self.agent.serialize());
        if computed_hash != self.hash {
            return Err(SignatureError::HashMismatch);
        }

        // 2. Verify signature
        let public_key = ed25519::PublicKey::from_bytes(&self.public_key)?;
        let signature = ed25519::Signature::from_bytes(&self.signature)?;
        public_key.verify(&self.hash, &signature)?;

        Ok(())
    }
}
```

### WASM Sandboxing

- Memory isolation: Each agent has separate linear memory
- Resource limits: CPU time, memory, fuel metering
- Controlled host access: Only allowed WIT imports available
- No direct system calls: All I/O through host functions

---

## Message Formats

### Protocol Buffers Schema

```protobuf
syntax = "proto3";
package fipa.v1;

message AgentId {
    string name = 1;
    repeated string addresses = 2;
}

enum Performative {
    PERFORMATIVE_UNSPECIFIED = 0;
    PERFORMATIVE_REQUEST = 1;
    PERFORMATIVE_INFORM = 2;
    PERFORMATIVE_QUERY_IF = 3;
    PERFORMATIVE_QUERY_REF = 4;
    PERFORMATIVE_CFP = 5;
    PERFORMATIVE_PROPOSE = 6;
    PERFORMATIVE_ACCEPT_PROPOSAL = 7;
    PERFORMATIVE_REJECT_PROPOSAL = 8;
    PERFORMATIVE_AGREE = 9;
    PERFORMATIVE_REFUSE = 10;
    PERFORMATIVE_FAILURE = 11;
    PERFORMATIVE_INFORM_DONE = 12;
    PERFORMATIVE_INFORM_RESULT = 13;
    PERFORMATIVE_NOT_UNDERSTOOD = 14;
    PERFORMATIVE_SUBSCRIBE = 15;
    PERFORMATIVE_CANCEL = 16;
}

enum ProtocolType {
    PROTOCOL_UNSPECIFIED = 0;
    PROTOCOL_REQUEST = 1;
    PROTOCOL_QUERY = 2;
    PROTOCOL_CONTRACT_NET = 3;
    PROTOCOL_SUBSCRIBE = 4;
    PROTOCOL_ENGLISH_AUCTION = 5;
    PROTOCOL_DUTCH_AUCTION = 6;
}

message AclMessage {
    string message_id = 1;
    Performative performative = 2;
    AgentId sender = 3;
    repeated AgentId receivers = 4;
    optional ProtocolType protocol = 5;
    optional string conversation_id = 6;
    optional string in_reply_to = 7;
    optional int64 reply_by = 8;
    optional string ontology = 9;
    bytes content = 10;
}

message MessageEnvelope {
    string source_node = 1;
    string target_node = 2;
    uint64 sequence = 3;
    int64 timestamp = 4;
    oneof payload {
        AclMessage message = 5;
        AgentMigration migration = 6;
        ConsensusMessage consensus = 7;
    }
}

service FipaAgentService {
    rpc SendMessage(AclMessage) returns (SendResponse);
    rpc FindAgent(FindAgentRequest) returns (FindAgentResponse);
    rpc MigrateAgent(AgentMigration) returns (MigrationResponse);
    rpc HealthCheck(HealthRequest) returns (HealthResponse);
}
```

---

## API Reference

### Public Rust API

```rust
// Create a new node
let config = NodeConfig::builder()
    .node_id("node-1")
    .listen_addr("/ip4/0.0.0.0/tcp/9000")
    .enable_mdns(true)
    .build();

let node = FipaNode::new(config).await?;

// Spawn an agent
let agent = node.spawn_agent(AgentConfig {
    id: AgentId::new("my-agent"),
    wasm_module: include_bytes!("agent.wasm").to_vec(),
    capabilities: Capabilities::default(),
}).await?;

// Send a message
agent.send(AclMessage {
    performative: Performative::Request,
    receiver: AgentId::new("other-agent"),
    content: b"perform task X".to_vec(),
    protocol: Some(ProtocolType::Request),
    ..Default::default()
}).await?;

// Find agents by service
let providers = node.find_service("data-processing").await?;

// Migrate agent
agent.migrate_to("node-2").await?;
```

---

## Deployment

### Node Requirements

- **CPU:** 2+ cores recommended
- **Memory:** 1GB minimum, 4GB+ for production
- **Disk:** 10GB+ for RocksDB storage
- **Network:** TCP ports for libp2p, gRPC

### Configuration

```toml
[node]
id = "node-1"
listen = ["/ip4/0.0.0.0/tcp/9000", "/ip4/0.0.0.0/udp/9000/quic-v1"]

[discovery]
mdns = true
kademlia = true
bootstrap = [
    "/ip4/192.168.1.100/tcp/9000/p2p/12D3KooW..."
]

[consensus]
enabled = true
raft_dir = "/var/lib/fipa/raft"
election_timeout_ms = 1000
heartbeat_interval_ms = 100

[wasm]
max_memory_bytes = 67108864  # 64 MB
max_execution_time_ms = 5000
fuel_limit = 1000000000

[metrics]
enabled = true
prometheus_port = 9090
```

### Docker

```dockerfile
FROM rust:1.83-slim as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/fipa-node /usr/local/bin/
EXPOSE 9000 9090
CMD ["fipa-node", "--config", "/etc/fipa/config.toml"]
```

---

## References

- [FIPA Specifications](http://www.fipa.org/repository/)
- [WebAssembly Component Model](https://github.com/WebAssembly/component-model)
- [WASI Preview 2](https://github.com/WebAssembly/WASI/blob/main/preview2/README.md)
- [libp2p](https://libp2p.io/)
- [Raft Consensus](https://raft.github.io/)
- [Actix Actor Framework](https://actix.rs/)
