# Plan: WASM Agent with MCP for Claude Integration

## Overview

Create a WASM agent that exposes FIPA agent platform capabilities to Claude via the Model Context Protocol (MCP). This enables Claude to interact with the multi-agent system - creating agents, sending messages, querying services, etc.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Claude Desktop                           │
│                              │                                   │
│                         MCP Client                               │
└──────────────────────────────┬──────────────────────────────────┘
                               │ JSON-RPC 2.0 (stdio/HTTP)
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                      MCP Server (Rust)                          │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                    MCP Tools Exposed                        │ │
│  │  • fipa_create_agent    • fipa_send_message                │ │
│  │  • fipa_destroy_agent   • fipa_query_agents                │ │
│  │  • fipa_list_agents     • fipa_search_services             │ │
│  │  • fipa_register_service • fipa_get_conversations          │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              │                                   │
│                         gRPC Client                              │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                    FIPA Agent Platform                           │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐            │
│  │   AMS   │  │   DF    │  │ Agent A │  │ Agent B │  ...       │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘            │
└─────────────────────────────────────────────────────────────────┘
```

## Components to Create

### 1. MCP Server Binary (`src/bin/fipa-mcp.rs`)

Standalone MCP server that connects to FIPA platform via gRPC.

```rust
// Dependencies
use mcp_protocol_sdk::{Server, Tool, Resource};
use tonic::transport::Channel;

#[tokio::main]
async fn main() {
    let fipa_client = FipaClient::connect("http://localhost:50051").await?;
    let server = McpServer::new(fipa_client);
    server.run_stdio().await?;
}
```

### 2. MCP Tools Definition (`src/mcp/tools.rs`)

| Tool | Description | Parameters |
|------|-------------|------------|
| `fipa_create_agent` | Create a new agent | name, wasm_path, capabilities |
| `fipa_destroy_agent` | Destroy an agent | agent_id |
| `fipa_list_agents` | List all agents | filter (optional) |
| `fipa_query_agent` | Get agent status | agent_id |
| `fipa_send_message` | Send ACL message | sender, receiver, performative, content |
| `fipa_search_services` | Search DF | service_type, protocol |
| `fipa_register_service` | Register service | agent_id, service_desc |
| `fipa_get_messages` | Get agent's messages | agent_id, limit |
| `fipa_start_conversation` | Start protocol | protocol_type, initiator, participants |

### 3. MCP Resources (`src/mcp/resources.rs`)

| Resource | URI Pattern | Description |
|----------|-------------|-------------|
| Agent list | `fipa://agents` | All agents |
| Agent detail | `fipa://agents/{id}` | Single agent |
| Services | `fipa://services` | DF registry |
| Messages | `fipa://agents/{id}/messages` | Agent mailbox |
| Platform | `fipa://platform` | Platform info |

### 4. WASM Agent Bridge (`examples/agents/claude-mcp/`)

Optional: A WASM agent that can be spawned by Claude to perform tasks.

## File Structure

```
src/
├── mcp/
│   ├── mod.rs           # MCP module
│   ├── server.rs        # MCP server implementation
│   ├── tools.rs         # Tool definitions
│   ├── resources.rs     # Resource definitions
│   └── prompts.rs       # Prompt templates
├── bin/
│   └── fipa-mcp.rs      # MCP server binary

examples/agents/claude-mcp/
├── Cargo.toml
├── src/
│   └── lib.rs           # WASM agent for Claude tasks
└── wit/
    └── fipa.wit         # WIT bindings
```

## Dependencies to Add

```toml
[dependencies]
# MCP SDK (choose one)
mcp-protocol-sdk = "0.1"   # Option A: Community SDK
# OR
pmcp = "1.4"               # Option B: High-performance

# JSON-RPC
jsonrpc-core = "18.0"
```

## MCP Configuration for Claude Desktop

`~/.config/claude/claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "fipa": {
      "command": "fipa-mcp",
      "args": ["--grpc-address", "localhost:50051"],
      "env": {}
    }
  }
}
```

## Implementation Steps

### Phase 1: Core MCP Server
1. [ ] Add MCP dependencies to Cargo.toml
2. [ ] Create `src/mcp/mod.rs` module structure
3. [ ] Implement MCP server with stdio transport
4. [ ] Add gRPC client connection to FIPA platform

### Phase 2: Tools Implementation
5. [ ] Implement `fipa_list_agents` tool
6. [ ] Implement `fipa_create_agent` tool
7. [ ] Implement `fipa_destroy_agent` tool
8. [ ] Implement `fipa_send_message` tool
9. [ ] Implement `fipa_search_services` tool
10. [ ] Implement remaining tools

### Phase 3: Resources
11. [ ] Implement agent list resource
12. [ ] Implement agent detail resource
13. [ ] Implement services resource
14. [ ] Implement message history resource

### Phase 4: WASM Agent (Optional)
15. [ ] Create claude-mcp WASM agent skeleton
16. [ ] Implement task execution behaviors
17. [ ] Add natural language parsing

### Phase 5: Testing & Documentation
18. [ ] Test with Claude Desktop
19. [ ] Add usage documentation
20. [ ] Create example prompts

## Example Interactions

### Claude Creating an Agent
```
User: Create a weather monitoring agent

Claude: I'll create a weather monitoring agent for you.
[Calls fipa_create_agent with name="weather-monitor", capabilities=[...]]

Done! Created agent "weather-monitor" with ID: agent-12345
```

### Claude Sending Messages
```
User: Ask the calculator agent to compute 42 * 17

Claude: I'll send a request to the calculator agent.
[Calls fipa_send_message with receiver="calculator",
 performative="REQUEST", content="(calculate (* 42 17))"]

The calculator agent responded: 714
```

### Claude Querying Services
```
User: What services are available?

Claude: Let me check the directory.
[Calls fipa_search_services]

Available services:
- weather-service (weather-agent): Weather data provider
- calculator-service (calc-agent): Mathematical operations
- translator-service (translator): Language translation
```

## Security Considerations

- MCP server should authenticate with FIPA platform
- Rate limiting on tool calls
- Agent creation permissions
- Message content validation

## Future Enhancements

- Streaming message notifications
- Conversation monitoring
- Agent behavior templates from Claude
- Natural language to ACL translation
- Multi-platform federation via MCP
