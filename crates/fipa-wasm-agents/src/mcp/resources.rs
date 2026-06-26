// mcp/resources.rs - MCP Resource Implementations
//
//! FIPA platform resources exposed via MCP.

use super::protocol::{ReadResourceResult, Resource, ResourceContent};
use super::tools::PlatformState;
use super::McpError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Resource handler trait
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    /// Resource URI pattern
    fn uri(&self) -> &str;

    /// Resource name
    fn name(&self) -> &str;

    /// Resource description
    fn description(&self) -> &str;

    /// MIME type
    fn mime_type(&self) -> &str {
        "application/json"
    }

    /// Read resource content
    async fn read(&self, uri: &str) -> Result<ReadResourceResult, McpError>;

    /// Convert to Resource definition
    fn to_resource(&self) -> Resource {
        Resource {
            uri: self.uri().to_string(),
            name: self.name().to_string(),
            description: Some(self.description().to_string()),
            mime_type: Some(self.mime_type().to_string()),
        }
    }
}

/// Resource registry
pub struct ResourceRegistry {
    resources: HashMap<String, Arc<dyn ResourceHandler>>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        Self {
            resources: HashMap::new(),
        }
    }

    /// Register a resource
    pub fn register<R: ResourceHandler + 'static>(&mut self, resource: R) {
        self.resources
            .insert(resource.uri().to_string(), Arc::new(resource));
    }

    /// Get a resource by URI
    pub fn get(&self, uri: &str) -> Option<Arc<dyn ResourceHandler>> {
        // Exact match first
        if let Some(r) = self.resources.get(uri) {
            return Some(r.clone());
        }

        // Try prefix match for parameterized URIs
        for (pattern, handler) in &self.resources {
            if uri.starts_with(pattern.trim_end_matches("/{id}")) {
                return Some(handler.clone());
            }
        }

        None
    }

    /// List all resources
    pub fn list(&self) -> Vec<Resource> {
        self.resources.values().map(|r| r.to_resource()).collect()
    }

    /// Read a resource
    pub async fn read(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        let resource = self
            .get(uri)
            .ok_or_else(|| McpError::ResourceNotFound(uri.to_string()))?;
        resource.read(uri).await
    }
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Resource Implementations
// ============================================================================

/// Agents list resource
pub struct AgentsResource {
    state: Arc<PlatformState>,
}

impl AgentsResource {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ResourceHandler for AgentsResource {
    fn uri(&self) -> &str {
        "fipa://agents"
    }

    fn name(&self) -> &str {
        "FIPA Agents"
    }

    fn description(&self) -> &str {
        "List of all agents registered on the FIPA platform"
    }

    async fn read(&self, _uri: &str) -> Result<ReadResourceResult, McpError> {
        let agents = self.state.agents.read().await;

        let agent_list: Vec<_> = agents
            .values()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "status": a.status,
                    "capabilities": a.capabilities,
                    "created_at": a.created_at
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&agent_list)
            .map_err(|e| McpError::Internal(e.to_string()))?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContent {
                uri: "fipa://agents".to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(json),
                blob: None,
            }],
        })
    }
}

/// Single agent resource
pub struct AgentDetailResource {
    state: Arc<PlatformState>,
}

impl AgentDetailResource {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ResourceHandler for AgentDetailResource {
    fn uri(&self) -> &str {
        "fipa://agents/{id}"
    }

    fn name(&self) -> &str {
        "Agent Details"
    }

    fn description(&self) -> &str {
        "Detailed information about a specific agent"
    }

    async fn read(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        // Extract agent ID from URI
        let agent_id = uri
            .strip_prefix("fipa://agents/")
            .ok_or_else(|| McpError::InvalidParams("Invalid agent URI".into()))?;

        let agents = self.state.agents.read().await;

        let agent = agents
            .get(agent_id)
            .or_else(|| agents.values().find(|a| a.name == agent_id))
            .ok_or_else(|| McpError::ResourceNotFound(format!("Agent '{}' not found", agent_id)))?;

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "id": agent.id,
            "name": agent.name,
            "status": agent.status,
            "capabilities": agent.capabilities,
            "created_at": agent.created_at
        }))
        .map_err(|e| McpError::Internal(e.to_string()))?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContent {
                uri: uri.to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(json),
                blob: None,
            }],
        })
    }
}

/// Services resource
pub struct ServicesResource {
    state: Arc<PlatformState>,
}

impl ServicesResource {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ResourceHandler for ServicesResource {
    fn uri(&self) -> &str {
        "fipa://services"
    }

    fn name(&self) -> &str {
        "FIPA Services"
    }

    fn description(&self) -> &str {
        "Services registered in the Directory Facilitator (DF)"
    }

    async fn read(&self, _uri: &str) -> Result<ReadResourceResult, McpError> {
        let services = self.state.services.read().await;

        let service_list: Vec<_> = services
            .values()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "agent_id": s.agent_id,
                    "service_type": s.service_type,
                    "protocols": s.protocols,
                    "description": s.description
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&service_list)
            .map_err(|e| McpError::Internal(e.to_string()))?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContent {
                uri: "fipa://services".to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(json),
                blob: None,
            }],
        })
    }
}

/// Platform info resource
pub struct PlatformResource {
    state: Arc<PlatformState>,
}

impl PlatformResource {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ResourceHandler for PlatformResource {
    fn uri(&self) -> &str {
        "fipa://platform"
    }

    fn name(&self) -> &str {
        "Platform Information"
    }

    fn description(&self) -> &str {
        "FIPA platform status and statistics"
    }

    async fn read(&self, _uri: &str) -> Result<ReadResourceResult, McpError> {
        let agents = self.state.agents.read().await;
        let services = self.state.services.read().await;
        let messages = self.state.messages.read().await;

        let active_agents = agents.values().filter(|a| a.status == "active").count();

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "name": "FIPA WASM Agent Platform",
            "version": env!("CARGO_PKG_VERSION"),
            "statistics": {
                "total_agents": agents.len(),
                "active_agents": active_agents,
                "total_services": services.len(),
                "total_messages": messages.len()
            },
            "capabilities": [
                "agent-management",
                "service-discovery",
                "acl-messaging",
                "fipa-protocols"
            ]
        }))
        .map_err(|e| McpError::Internal(e.to_string()))?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContent {
                uri: "fipa://platform".to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(json),
                blob: None,
            }],
        })
    }
}

/// Messages resource
pub struct MessagesResource {
    state: Arc<PlatformState>,
}

impl MessagesResource {
    pub fn new(state: Arc<PlatformState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ResourceHandler for MessagesResource {
    fn uri(&self) -> &str {
        "fipa://messages"
    }

    fn name(&self) -> &str {
        "Message History"
    }

    fn description(&self) -> &str {
        "Recent messages on the platform"
    }

    async fn read(&self, _uri: &str) -> Result<ReadResourceResult, McpError> {
        let messages = self.state.messages.read().await;

        let message_list: Vec<_> = messages
            .iter()
            .rev()
            .take(50)
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "sender": m.sender,
                    "receiver": m.receiver,
                    "performative": m.performative,
                    "content": m.content,
                    "timestamp": m.timestamp
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&message_list)
            .map_err(|e| McpError::Internal(e.to_string()))?;

        Ok(ReadResourceResult {
            contents: vec![ResourceContent {
                uri: "fipa://messages".to_string(),
                mime_type: Some("application/json".to_string()),
                text: Some(json),
                blob: None,
            }],
        })
    }
}

/// Create a registry with all FIPA resources
pub fn create_fipa_resources(state: Arc<PlatformState>) -> ResourceRegistry {
    let mut registry = ResourceRegistry::new();

    registry.register(AgentsResource::new(state.clone()));
    registry.register(AgentDetailResource::new(state.clone()));
    registry.register(ServicesResource::new(state.clone()));
    registry.register(PlatformResource::new(state.clone()));
    registry.register(MessagesResource::new(state.clone()));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agents_resource() {
        let state = Arc::new(PlatformState::new());

        // Add an agent
        {
            let mut agents = state.agents.write().await;
            agents.insert(
                "agent-1".to_string(),
                super::super::tools::AgentInfo {
                    id: "agent-1".to_string(),
                    name: "test-agent".to_string(),
                    status: "active".to_string(),
                    capabilities: vec!["test".to_string()],
                    created_at: "2024-01-01".to_string(),
                },
            );
        }

        let resource = AgentsResource::new(state);
        let result = resource.read("fipa://agents").await.unwrap();

        assert_eq!(result.contents.len(), 1);
        let text = result.contents[0].text.as_ref().unwrap();
        assert!(text.contains("test-agent"));
    }

    #[tokio::test]
    async fn test_platform_resource() {
        let state = Arc::new(PlatformState::new());
        let resource = PlatformResource::new(state);

        let result = resource.read("fipa://platform").await.unwrap();
        assert_eq!(result.contents.len(), 1);

        let text = result.contents[0].text.as_ref().unwrap();
        assert!(text.contains("FIPA WASM Agent Platform"));
    }

    #[tokio::test]
    async fn test_resource_registry() {
        let state = Arc::new(PlatformState::new());
        let registry = create_fipa_resources(state);

        let resources = registry.list();
        assert!(resources.len() >= 5);

        assert!(registry.get("fipa://agents").is_some());
        assert!(registry.get("fipa://platform").is_some());
    }
}
