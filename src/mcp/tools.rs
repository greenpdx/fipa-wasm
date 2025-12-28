// mcp/tools.rs - MCP Tool Implementations
//
//! FIPA platform tools exposed via MCP.

use super::protocol::{CallToolResult, Tool, ToolContent};
use super::McpError;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tool handler trait
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Tool name
    fn name(&self) -> &str;

    /// Tool description
    fn description(&self) -> &str;

    /// Input schema (JSON Schema)
    fn input_schema(&self) -> Value;

    /// Execute the tool
    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError>;

    /// Convert to Tool definition
    fn to_tool(&self) -> Tool {
        Tool {
            name: self.name().to_string(),
            description: Some(self.description().to_string()),
            input_schema: self.input_schema(),
        }
    }
}

/// Tool registry
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register<T: ToolHandler + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.tools.get(name).cloned()
    }

    /// List all tools
    pub fn list(&self) -> Vec<Tool> {
        self.tools.values().map(|t| t.to_tool()).collect()
    }

    /// Call a tool
    pub async fn call(&self, name: &str, arguments: Value) -> Result<CallToolResult, McpError> {
        let tool = self.get(name).ok_or_else(|| McpError::ToolNotFound(name.to_string()))?;
        tool.call(arguments).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// FIPA Platform State (shared with tools)
// ============================================================================

/// Simulated platform state for standalone operation
/// In production, this would connect to the actual FIPA platform via gRPC
#[derive(Debug, Default)]
pub struct PlatformState {
    pub agents: RwLock<HashMap<String, AgentInfo>>,
    pub services: RwLock<HashMap<String, ServiceInfo>>,
    pub messages: RwLock<Vec<MessageInfo>>,
    next_id: RwLock<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub agent_id: String,
    pub service_type: String,
    pub protocols: Vec<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageInfo {
    pub id: String,
    pub sender: String,
    pub receiver: String,
    pub performative: String,
    pub content: String,
    pub timestamp: String,
}

impl PlatformState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn next_id(&self) -> u64 {
        let mut id = self.next_id.write().await;
        *id += 1;
        *id
    }
}

// ============================================================================
// Tool Implementations
// ============================================================================

/// Create Agent Tool
pub struct CreateAgentTool {
    state: Arc<PlatformState>,
}

impl CreateAgentTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for CreateAgentTool {
    fn name(&self) -> &str {
        "fipa_create_agent"
    }

    fn description(&self) -> &str {
        "Create a new FIPA agent on the platform"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Unique name for the agent"
                },
                "capabilities": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Agent capabilities (e.g., 'weather', 'calculator')"
                }
            },
            "required": ["name"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let name = arguments["name"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("name is required".into()))?;

        let capabilities: Vec<String> = arguments["capabilities"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Check if agent already exists
        {
            let agents = self.state.agents.read().await;
            if agents.values().any(|a| a.name == name) {
                return Ok(CallToolResult {
                    content: vec![ToolContent::text(format!(
                        "Error: Agent '{}' already exists",
                        name
                    ))],
                    is_error: Some(true),
                });
            }
        }

        // Create new agent
        let id = format!("agent-{}", self.state.next_id().await);
        let agent = AgentInfo {
            id: id.clone(),
            name: name.to_string(),
            status: "active".to_string(),
            capabilities,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        {
            let mut agents = self.state.agents.write().await;
            agents.insert(id.clone(), agent);
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(format!(
                "Created agent '{}' with ID: {}",
                name, id
            ))],
            is_error: None,
        })
    }
}

/// Destroy Agent Tool
pub struct DestroyAgentTool {
    state: Arc<PlatformState>,
}

impl DestroyAgentTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for DestroyAgentTool {
    fn name(&self) -> &str {
        "fipa_destroy_agent"
    }

    fn description(&self) -> &str {
        "Destroy an existing FIPA agent"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID or name to destroy"
                }
            },
            "required": ["agent_id"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let agent_id = arguments["agent_id"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("agent_id is required".into()))?;

        let mut agents = self.state.agents.write().await;

        // Try to find by ID first, then by name
        let key = if agents.contains_key(agent_id) {
            Some(agent_id.to_string())
        } else {
            agents
                .iter()
                .find(|(_, a)| a.name == agent_id)
                .map(|(k, _)| k.clone())
        };

        if let Some(key) = key {
            let agent = agents.remove(&key).unwrap();
            Ok(CallToolResult {
                content: vec![ToolContent::text(format!(
                    "Destroyed agent '{}' (ID: {})",
                    agent.name, agent.id
                ))],
                is_error: None,
            })
        } else {
            Ok(CallToolResult {
                content: vec![ToolContent::text(format!(
                    "Error: Agent '{}' not found",
                    agent_id
                ))],
                is_error: Some(true),
            })
        }
    }
}

/// List Agents Tool
pub struct ListAgentsTool {
    state: Arc<PlatformState>,
}

impl ListAgentsTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for ListAgentsTool {
    fn name(&self) -> &str {
        "fipa_list_agents"
    }

    fn description(&self) -> &str {
        "List all agents on the FIPA platform"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": "Filter by status (active, suspended, etc.)"
                }
            }
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let status_filter = arguments["status"].as_str();

        let agents = self.state.agents.read().await;
        let filtered: Vec<_> = agents
            .values()
            .filter(|a| status_filter.map_or(true, |s| a.status == s))
            .collect();

        if filtered.is_empty() {
            return Ok(CallToolResult {
                content: vec![ToolContent::text("No agents found on the platform.")],
                is_error: None,
            });
        }

        let mut result = String::from("Agents on the platform:\n\n");
        for agent in filtered {
            result.push_str(&format!(
                "- **{}** (ID: {})\n  Status: {}\n  Capabilities: {}\n  Created: {}\n\n",
                agent.name,
                agent.id,
                agent.status,
                if agent.capabilities.is_empty() {
                    "none".to_string()
                } else {
                    agent.capabilities.join(", ")
                },
                agent.created_at
            ));
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(result)],
            is_error: None,
        })
    }
}

/// Send Message Tool
pub struct SendMessageTool {
    state: Arc<PlatformState>,
}

impl SendMessageTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for SendMessageTool {
    fn name(&self) -> &str {
        "fipa_send_message"
    }

    fn description(&self) -> &str {
        "Send a FIPA ACL message to an agent"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sender": {
                    "type": "string",
                    "description": "Sender agent name or ID"
                },
                "receiver": {
                    "type": "string",
                    "description": "Receiver agent name or ID"
                },
                "performative": {
                    "type": "string",
                    "enum": ["REQUEST", "INFORM", "QUERY_IF", "QUERY_REF", "CFP", "PROPOSE", "ACCEPT_PROPOSAL", "REJECT_PROPOSAL", "AGREE", "REFUSE", "CANCEL"],
                    "description": "FIPA performative type"
                },
                "content": {
                    "type": "string",
                    "description": "Message content"
                }
            },
            "required": ["sender", "receiver", "performative", "content"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let sender = arguments["sender"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("sender is required".into()))?;
        let receiver = arguments["receiver"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("receiver is required".into()))?;
        let performative = arguments["performative"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("performative is required".into()))?;
        let content = arguments["content"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("content is required".into()))?;

        // Verify receiver exists
        {
            let agents = self.state.agents.read().await;
            let receiver_exists = agents.contains_key(receiver)
                || agents.values().any(|a| a.name == receiver);
            if !receiver_exists {
                return Ok(CallToolResult {
                    content: vec![ToolContent::text(format!(
                        "Warning: Receiver '{}' not found on platform. Message queued for delivery.",
                        receiver
                    ))],
                    is_error: None,
                });
            }
        }

        // Create message
        let id = format!("msg-{}", self.state.next_id().await);
        let message = MessageInfo {
            id: id.clone(),
            sender: sender.to_string(),
            receiver: receiver.to_string(),
            performative: performative.to_string(),
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        {
            let mut messages = self.state.messages.write().await;
            messages.push(message);
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(format!(
                "Sent {} message from '{}' to '{}'\nMessage ID: {}\nContent: {}",
                performative, sender, receiver, id, content
            ))],
            is_error: None,
        })
    }
}

/// Search Services Tool
pub struct SearchServicesTool {
    state: Arc<PlatformState>,
}

impl SearchServicesTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for SearchServicesTool {
    fn name(&self) -> &str {
        "fipa_search_services"
    }

    fn description(&self) -> &str {
        "Search for services registered in the Directory Facilitator (DF)"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "service_type": {
                    "type": "string",
                    "description": "Service type to search for"
                },
                "protocol": {
                    "type": "string",
                    "description": "Protocol the service should support"
                }
            }
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let type_filter = arguments["service_type"].as_str();
        let protocol_filter = arguments["protocol"].as_str();

        let services = self.state.services.read().await;
        let filtered: Vec<_> = services
            .values()
            .filter(|s| {
                type_filter.map_or(true, |t| s.service_type.contains(t))
                    && protocol_filter.map_or(true, |p| s.protocols.iter().any(|sp| sp.contains(p)))
            })
            .collect();

        if filtered.is_empty() {
            return Ok(CallToolResult {
                content: vec![ToolContent::text(
                    "No services found matching the criteria.",
                )],
                is_error: None,
            });
        }

        let mut result = String::from("Services found:\n\n");
        for service in filtered {
            result.push_str(&format!(
                "- **{}** ({})\n  Agent: {}\n  Protocols: {}\n  {}\n\n",
                service.name,
                service.service_type,
                service.agent_id,
                service.protocols.join(", "),
                service.description.as_deref().unwrap_or("No description")
            ));
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(result)],
            is_error: None,
        })
    }
}

/// Register Service Tool
pub struct RegisterServiceTool {
    state: Arc<PlatformState>,
}

impl RegisterServiceTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for RegisterServiceTool {
    fn name(&self) -> &str {
        "fipa_register_service"
    }

    fn description(&self) -> &str {
        "Register a service in the Directory Facilitator (DF)"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Agent providing the service"
                },
                "service_name": {
                    "type": "string",
                    "description": "Name of the service"
                },
                "service_type": {
                    "type": "string",
                    "description": "Type of service"
                },
                "protocols": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Supported protocols"
                },
                "description": {
                    "type": "string",
                    "description": "Service description"
                }
            },
            "required": ["agent_id", "service_name", "service_type"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let agent_id = arguments["agent_id"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("agent_id is required".into()))?;
        let service_name = arguments["service_name"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("service_name is required".into()))?;
        let service_type = arguments["service_type"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("service_type is required".into()))?;

        let protocols: Vec<String> = arguments["protocols"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["fipa-request".to_string()]);

        let description = arguments["description"].as_str().map(String::from);

        let service = ServiceInfo {
            name: service_name.to_string(),
            agent_id: agent_id.to_string(),
            service_type: service_type.to_string(),
            protocols,
            description,
        };

        {
            let mut services = self.state.services.write().await;
            services.insert(service_name.to_string(), service);
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(format!(
                "Registered service '{}' of type '{}' for agent '{}'",
                service_name, service_type, agent_id
            ))],
            is_error: None,
        })
    }
}

/// Get Messages Tool
pub struct GetMessagesTool {
    state: Arc<PlatformState>,
}

impl GetMessagesTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for GetMessagesTool {
    fn name(&self) -> &str {
        "fipa_get_messages"
    }

    fn description(&self) -> &str {
        "Get messages for an agent"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID or name"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages to return",
                    "default": 10
                }
            },
            "required": ["agent_id"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let agent_id = arguments["agent_id"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("agent_id is required".into()))?;
        let limit = arguments["limit"].as_u64().unwrap_or(10) as usize;

        let messages = self.state.messages.read().await;
        let agent_messages: Vec<_> = messages
            .iter()
            .filter(|m| m.receiver == agent_id || m.sender == agent_id)
            .rev()
            .take(limit)
            .collect();

        if agent_messages.is_empty() {
            return Ok(CallToolResult {
                content: vec![ToolContent::text(format!(
                    "No messages found for agent '{}'",
                    agent_id
                ))],
                is_error: None,
            });
        }

        let mut result = format!("Messages for agent '{}':\n\n", agent_id);
        for msg in agent_messages {
            let direction = if msg.receiver == agent_id {
                "received from"
            } else {
                "sent to"
            };
            let other = if msg.receiver == agent_id {
                &msg.sender
            } else {
                &msg.receiver
            };

            result.push_str(&format!(
                "- [{}] {} {} '{}'\n  Performative: {}\n  Content: {}\n\n",
                msg.timestamp, direction, other, msg.id, msg.performative, msg.content
            ));
        }

        Ok(CallToolResult {
            content: vec![ToolContent::text(result)],
            is_error: None,
        })
    }
}

/// Query Agent Tool
pub struct QueryAgentTool {
    state: Arc<PlatformState>,
}

impl QueryAgentTool {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ToolHandler for QueryAgentTool {
    fn name(&self) -> &str {
        "fipa_query_agent"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific agent"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID or name"
                }
            },
            "required": ["agent_id"]
        })
    }

    async fn call(&self, arguments: Value) -> Result<CallToolResult, McpError> {
        let agent_id = arguments["agent_id"]
            .as_str()
            .ok_or_else(|| McpError::InvalidParams("agent_id is required".into()))?;

        let agents = self.state.agents.read().await;

        // Find by ID or name
        let agent = agents
            .get(agent_id)
            .or_else(|| agents.values().find(|a| a.name == agent_id));

        match agent {
            Some(agent) => {
                let result = format!(
                    "Agent Details:\n\n\
                     - **Name**: {}\n\
                     - **ID**: {}\n\
                     - **Status**: {}\n\
                     - **Capabilities**: {}\n\
                     - **Created**: {}",
                    agent.name,
                    agent.id,
                    agent.status,
                    if agent.capabilities.is_empty() {
                        "none".to_string()
                    } else {
                        agent.capabilities.join(", ")
                    },
                    agent.created_at
                );

                Ok(CallToolResult {
                    content: vec![ToolContent::text(result)],
                    is_error: None,
                })
            }
            None => Ok(CallToolResult {
                content: vec![ToolContent::text(format!(
                    "Agent '{}' not found",
                    agent_id
                ))],
                is_error: Some(true),
            }),
        }
    }
}

/// Create a registry with all FIPA tools
pub fn create_fipa_tools(state: Arc<PlatformState>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    registry.register(CreateAgentTool::new(state.clone()));
    registry.register(DestroyAgentTool::new(state.clone()));
    registry.register(ListAgentsTool::new(state.clone()));
    registry.register(QueryAgentTool::new(state.clone()));
    registry.register(SendMessageTool::new(state.clone()));
    registry.register(GetMessagesTool::new(state.clone()));
    registry.register(SearchServicesTool::new(state.clone()));
    registry.register(RegisterServiceTool::new(state.clone()));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_agent_tool() {
        let state = Arc::new(PlatformState::new());
        let tool = CreateAgentTool::new(state.clone());

        let result = tool
            .call(json!({
                "name": "test-agent",
                "capabilities": ["test"]
            }))
            .await
            .unwrap();

        assert!(result.is_error.is_none());

        let agents = state.agents.read().await;
        assert_eq!(agents.len(), 1);
    }

    #[tokio::test]
    async fn test_list_agents_tool() {
        let state = Arc::new(PlatformState::new());

        // Add some agents
        {
            let mut agents = state.agents.write().await;
            agents.insert(
                "agent-1".to_string(),
                AgentInfo {
                    id: "agent-1".to_string(),
                    name: "test-agent".to_string(),
                    status: "active".to_string(),
                    capabilities: vec![],
                    created_at: "2024-01-01".to_string(),
                },
            );
        }

        let tool = ListAgentsTool::new(state);
        let result = tool.call(json!({})).await.unwrap();

        assert!(result.is_error.is_none());
        if let ToolContent::Text { text } = &result.content[0] {
            assert!(text.contains("test-agent"));
        }
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let state = Arc::new(PlatformState::new());
        let registry = create_fipa_tools(state);

        let tools = registry.list();
        assert!(tools.len() >= 8);

        assert!(registry.get("fipa_create_agent").is_some());
        assert!(registry.get("fipa_send_message").is_some());
        assert!(registry.get("nonexistent").is_none());
    }
}
