// bin/fipa_mcp.rs - FIPA MCP Server Binary
//
//! MCP Server for Claude integration with FIPA agent platform.
//!
//! This binary provides an MCP (Model Context Protocol) server that exposes
//! FIPA agent platform capabilities to Claude and other MCP-compatible clients.
//!
//! # Usage
//!
//! ```bash
//! # Run the MCP server (typically launched by Claude Desktop)
//! fipa-mcp
//!
//! # With debug logging
//! RUST_LOG=debug fipa-mcp
//! ```
//!
//! # Claude Desktop Configuration
//!
//! Add to `~/.config/claude/claude_desktop_config.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "fipa": {
//!       "command": "fipa-mcp",
//!       "args": []
//!     }
//!   }
//! }
//! ```

use clap::Parser;
use fipa_wasm_agents::mcp::{McpConfig, McpServer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// FIPA MCP Server - Model Context Protocol server for Claude integration
#[derive(Parser, Debug)]
#[command(name = "fipa-mcp")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Server name to advertise
    #[arg(long, default_value = "fipa-mcp")]
    name: String,

    /// Enable JSON logging format
    #[arg(long)]
    json_logs: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize logging (to stderr to not interfere with MCP stdio)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if args.json_logs {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json().with_writer(std::io::stderr))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    }

    info!("Starting FIPA MCP Server v{}", env!("CARGO_PKG_VERSION"));

    let config = McpConfig {
        name: args.name,
        ..Default::default()
    };

    let server = McpServer::new(config);
    server.run_stdio().await?;

    Ok(())
}
