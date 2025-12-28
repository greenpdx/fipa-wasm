// mcp/mod.rs - Model Context Protocol Server
//
//! MCP (Model Context Protocol) Server for Claude Integration
//!
//! This module provides an MCP server that exposes FIPA agent platform
//! capabilities to Claude and other MCP-compatible AI assistants.
//!
//! # Architecture
//!
//! ```text
//! Claude Desktop ──MCP (JSON-RPC)──> fipa-mcp ──gRPC──> FIPA Platform
//! ```
//!
//! # Features
//!
//! - **Tools**: Create/destroy agents, send messages, search services
//! - **Resources**: Agent list, service directory, platform info
//! - **Transport**: stdio for Claude Desktop integration
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::mcp::{McpServer, McpConfig};
//!
//! let config = McpConfig {
//!     grpc_address: "http://localhost:50051".to_string(),
//!     ..Default::default()
//! };
//!
//! let server = McpServer::new(config).await?;
//! server.run_stdio().await?;
//! ```

#[cfg(feature = "mcp")]
pub mod protocol;
#[cfg(feature = "mcp")]
pub mod server;
#[cfg(feature = "mcp")]
pub mod tools;
#[cfg(feature = "mcp")]
pub mod resources;
#[cfg(feature = "mcp")]
pub mod transport;

#[cfg(feature = "mcp")]
pub use protocol::*;
#[cfg(feature = "mcp")]
pub use server::{McpServer, McpConfig};
#[cfg(feature = "mcp")]
pub use tools::ToolRegistry;
#[cfg(feature = "mcp")]
pub use resources::ResourceRegistry;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// MCP errors
#[derive(Debug, Error)]
pub enum McpError {
    #[error("JSON-RPC error: {0}")]
    JsonRpc(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("gRPC error: {0}")]
    Grpc(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// MCP server information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "fipa-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// MCP capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourcesCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_info_default() {
        let info = ServerInfo::default();
        assert_eq!(info.name, "fipa-mcp");
        assert!(!info.version.is_empty());
    }

    #[test]
    fn test_capabilities_serialization() {
        let caps = ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: Some(true) }),
            resources: Some(ResourcesCapability {
                subscribe: Some(true),
                list_changed: Some(true),
            }),
            prompts: None,
        };

        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains("tools"));
        assert!(json.contains("resources"));
        assert!(!json.contains("prompts"));
    }
}
