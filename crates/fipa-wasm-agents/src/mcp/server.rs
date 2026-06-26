// mcp/server.rs - MCP Server Implementation
//
//! Main MCP server that handles requests and manages tools/resources.

use super::protocol::*;
use super::resources::{create_fipa_resources, ResourceRegistry};
use super::tools::{create_fipa_tools, PlatformState, ToolRegistry};
use super::transport::{StdioTransport, TransportMessage};
use super::{McpError, ServerCapabilities, ServerInfo, ToolsCapability, ResourcesCapability};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// MCP server configuration
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Server name
    pub name: String,

    /// Server version
    pub version: String,

    /// Protocol version to advertise
    pub protocol_version: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            name: "fipa-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: "2024-11-05".to_string(),
        }
    }
}

/// MCP Server
pub struct McpServer {
    config: McpConfig,
    state: Arc<PlatformState>,
    tools: ToolRegistry,
    resources: ResourceRegistry,
    initialized: bool,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(config: McpConfig) -> Self {
        let state = Arc::new(PlatformState::new());
        let tools = create_fipa_tools(state.clone());
        let resources = create_fipa_resources(state.clone());

        Self {
            config,
            state,
            tools,
            resources,
            initialized: false,
        }
    }

    /// Create with default config
    pub fn with_defaults() -> Self {
        Self::new(McpConfig::default())
    }

    /// Get server capabilities
    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: Some(false),
            }),
            resources: Some(ResourcesCapability {
                subscribe: Some(false),
                list_changed: Some(false),
            }),
            prompts: None,
        }
    }

    /// Get server info
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            name: self.config.name.clone(),
            version: self.config.version.clone(),
        }
    }

    /// Handle a JSON-RPC request
    pub async fn handle_request(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!("Handling request: {} (id: {:?})", request.method, request.id);

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await,
            "initialized" => {
                // This is actually a notification, but some clients send it as request
                self.initialized = true;
                Ok(json!({}))
            }
            "tools/list" => self.handle_list_tools().await,
            "tools/call" => self.handle_call_tool(request.params).await,
            "resources/list" => self.handle_list_resources().await,
            "resources/read" => self.handle_read_resource(request.params).await,
            "ping" => Ok(json!({})),
            _ => Err(McpError::JsonRpc(format!(
                "Unknown method: {}",
                request.method
            ))),
        };

        match result {
            Ok(value) => JsonRpcResponse::success(request.id, value),
            Err(e) => {
                error!("Request error: {}", e);
                JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::internal_error(&e.to_string()),
                )
            }
        }
    }

    /// Handle initialize request
    async fn handle_initialize(&mut self, params: Option<Value>) -> Result<Value, McpError> {
        let _params: InitializeParams = params
            .map(|p| serde_json::from_value(p))
            .transpose()
            .map_err(|e| McpError::InvalidParams(e.to_string()))?
            .unwrap_or_else(|| InitializeParams {
                protocol_version: self.config.protocol_version.clone(),
                capabilities: ClientCapabilities::default(),
                client_info: ClientInfo {
                    name: "unknown".to_string(),
                    version: "unknown".to_string(),
                },
            });

        info!("Initializing MCP server: {}", self.config.name);

        let result = InitializeResult {
            protocol_version: self.config.protocol_version.clone(),
            capabilities: self.capabilities(),
            server_info: self.server_info(),
        };

        Ok(serde_json::to_value(result)?)
    }

    /// Handle tools/list request
    async fn handle_list_tools(&self) -> Result<Value, McpError> {
        let tools = self.tools.list();
        let result = ListToolsResult { tools };
        Ok(serde_json::to_value(result)?)
    }

    /// Handle tools/call request
    async fn handle_call_tool(&self, params: Option<Value>) -> Result<Value, McpError> {
        let params: CallToolParams = params
            .ok_or_else(|| McpError::InvalidParams("params required".into()))
            .and_then(|p| {
                serde_json::from_value(p).map_err(|e| McpError::InvalidParams(e.to_string()))
            })?;

        debug!("Calling tool: {}", params.name);

        let result = self.tools.call(&params.name, params.arguments).await?;
        Ok(serde_json::to_value(result)?)
    }

    /// Handle resources/list request
    async fn handle_list_resources(&self) -> Result<Value, McpError> {
        let resources = self.resources.list();
        let result = ListResourcesResult { resources };
        Ok(serde_json::to_value(result)?)
    }

    /// Handle resources/read request
    async fn handle_read_resource(&self, params: Option<Value>) -> Result<Value, McpError> {
        let params: ReadResourceParams = params
            .ok_or_else(|| McpError::InvalidParams("params required".into()))
            .and_then(|p| {
                serde_json::from_value(p).map_err(|e| McpError::InvalidParams(e.to_string()))
            })?;

        debug!("Reading resource: {}", params.uri);

        let result = self.resources.read(&params.uri).await?;
        Ok(serde_json::to_value(result)?)
    }

    /// Handle a notification
    pub async fn handle_notification(&mut self, notification: JsonRpcNotification) {
        debug!("Handling notification: {}", notification.method);

        match notification.method.as_str() {
            "initialized" => {
                self.initialized = true;
                info!("Client initialized");
            }
            "cancelled" => {
                // Handle cancellation
                if let Some(params) = notification.params {
                    if let Some(request_id) = params.get("requestId") {
                        warn!("Request cancelled: {:?}", request_id);
                    }
                }
            }
            _ => {
                debug!("Ignoring unknown notification: {}", notification.method);
            }
        }
    }

    /// Run the server on stdio
    pub async fn run_stdio(mut self) -> Result<(), McpError> {
        info!("Starting MCP server on stdio");

        let (transport, mut rx) = StdioTransport::new();

        while let Some(msg) = rx.recv().await {
            match msg {
                TransportMessage::Request(request) => {
                    let response = self.handle_request(request).await;
                    transport.send_response(response).await?;
                }
                TransportMessage::Notification(notification) => {
                    self.handle_notification(notification).await;
                }
            }
        }

        info!("MCP server shutting down");
        Ok(())
    }

    /// Get the platform state (for testing/extension)
    pub fn state(&self) -> Arc<PlatformState> {
        self.state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let server = McpServer::with_defaults();
        assert!(!server.initialized);
    }

    #[tokio::test]
    async fn test_initialize() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(
            1,
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0"
                }
            })),
        );

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());

        let result: InitializeResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.server_info.name, "fipa-mcp");
    }

    #[tokio::test]
    async fn test_list_tools() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(1, "tools/list", None);
        let response = server.handle_request(request).await;

        assert!(response.result.is_some());
        let result: ListToolsResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.tools.len() >= 8);

        // Check for expected tools
        let tool_names: Vec<_> = result.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"fipa_create_agent"));
        assert!(tool_names.contains(&"fipa_send_message"));
    }

    #[tokio::test]
    async fn test_call_tool() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(
            1,
            "tools/call",
            Some(json!({
                "name": "fipa_create_agent",
                "arguments": {
                    "name": "test-agent",
                    "capabilities": ["test"]
                }
            })),
        );

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());

        // Verify agent was created
        let agents = server.state.agents.read().await;
        assert_eq!(agents.len(), 1);
    }

    #[tokio::test]
    async fn test_list_resources() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(1, "resources/list", None);
        let response = server.handle_request(request).await;

        assert!(response.result.is_some());
        let result: ListResourcesResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(result.resources.len() >= 5);
    }

    #[tokio::test]
    async fn test_read_resource() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(
            1,
            "resources/read",
            Some(json!({
                "uri": "fipa://platform"
            })),
        );

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());

        let result: ReadResourceResult =
            serde_json::from_value(response.result.unwrap()).unwrap();
        assert_eq!(result.contents.len(), 1);
        assert!(result.contents[0].text.is_some());
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let mut server = McpServer::with_defaults();

        let request = JsonRpcRequest::new(1, "unknown/method", None);
        let response = server.handle_request(request).await;

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32603);
    }
}
