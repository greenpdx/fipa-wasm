// bin/cli.rs - FIPA CLI Tool
//
//! FIPA Command Line Interface
//!
//! A comprehensive CLI for interacting with FIPA agent platforms.
//!
//! # Usage
//!
//! ```bash
//! # List agents on a node
//! fipa-cli agents list
//!
//! # Create a new agent
//! fipa-cli agents create my-agent --wasm ./agent.wasm
//!
//! # Send a message
//! fipa-cli messages send --to target-agent --content "hello"
//!
//! # Search for services
//! fipa-cli services search calculator
//! ```

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use std::time::Duration;
use tonic::transport::Channel;

// Import generated proto types
pub mod fipa {
    pub mod v1 {
        tonic::include_proto!("fipa.v1");
    }
}

use fipa::v1::{
    fipa_agent_service_client::FipaAgentServiceClient,
    AclMessage, FindAgentRequest, FindServiceRequest, HealthCheckRequest,
    NodeInfoRequest, Performative, ProtocolType, AgentId,
};

/// FIPA CLI Tool
#[derive(Parser, Debug)]
#[command(name = "fipa-cli")]
#[command(author = "FIPA WASM Agents")]
#[command(version)]
#[command(about = "FIPA Agent System CLI - Manage agents, services, and messages")]
#[command(long_about = None)]
struct Args {
    /// Node address to connect to (host:port)
    #[arg(short, long, default_value = "http://localhost:9000", global = true)]
    node: String,

    /// Connection timeout in seconds
    #[arg(long, default_value = "10", global = true)]
    timeout: u64,

    /// Output format (text, json)
    #[arg(short, long, default_value = "text", global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Agent management commands
    #[command(subcommand)]
    Agents(AgentCommands),

    /// Service discovery commands
    #[command(subcommand)]
    Services(ServiceCommands),

    /// Message sending commands
    #[command(subcommand)]
    Messages(MessageCommands),

    /// Node and cluster commands
    #[command(subcommand)]
    Nodes(NodeCommands),

    /// Quick status check
    Status,

    /// Interactive shell mode
    Shell,
}

#[derive(Subcommand, Debug)]
enum AgentCommands {
    /// List all agents on the node
    List,

    /// Create a new agent
    Create {
        /// Agent name
        name: String,

        /// Path to WASM module
        #[arg(short, long)]
        wasm: PathBuf,

        /// Owner name (optional)
        #[arg(short, long)]
        owner: Option<String>,
    },

    /// Destroy an agent
    Destroy {
        /// Agent name
        name: String,
    },

    /// Get agent status
    Status {
        /// Agent name
        name: String,
    },

    /// Suspend an agent
    Suspend {
        /// Agent name
        name: String,
    },

    /// Resume a suspended agent
    Resume {
        /// Agent name
        name: String,
    },

    /// Migrate an agent to another node
    Migrate {
        /// Agent name
        name: String,

        /// Target node address
        #[arg(short, long)]
        target: String,
    },

    /// Clone an agent to another node
    Clone {
        /// Agent name
        name: String,

        /// Target node address
        #[arg(short, long)]
        target: String,

        /// New agent name (optional)
        #[arg(long)]
        new_name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceCommands {
    /// List all registered services
    List,

    /// Search for services by name
    Search {
        /// Service name pattern
        name: String,

        /// Protocol filter
        #[arg(short, long)]
        protocol: Option<String>,

        /// Ontology filter
        #[arg(short, long)]
        ontology: Option<String>,
    },

    /// Register a service (for testing)
    Register {
        /// Service name
        name: String,

        /// Service description
        #[arg(short, long)]
        description: Option<String>,

        /// Agent providing the service
        #[arg(short, long)]
        agent: String,
    },

    /// Deregister a service
    Deregister {
        /// Service name
        name: String,

        /// Agent name
        #[arg(short, long)]
        agent: String,
    },
}

#[derive(Subcommand, Debug)]
enum MessageCommands {
    /// Send a message to an agent
    Send {
        /// Target agent name
        #[arg(short, long)]
        to: String,

        /// Message content
        #[arg(short, long)]
        content: String,

        /// Performative (request, inform, query-ref, cfp, etc.)
        #[arg(short, long, default_value = "request")]
        performative: String,

        /// Protocol (request, query, contract-net, etc.)
        #[arg(long, default_value = "request")]
        protocol: String,

        /// Sender name (defaults to "cli")
        #[arg(short, long, default_value = "cli")]
        from: String,
    },

    /// Subscribe to messages for an agent (sniffing)
    Sniff {
        /// Agent name to sniff
        agent: String,

        /// Duration in seconds (0 = indefinite)
        #[arg(short, long, default_value = "0")]
        duration: u64,
    },

    /// Send a file as message content
    SendFile {
        /// Target agent name
        #[arg(short, long)]
        to: String,

        /// File path
        #[arg(short, long)]
        file: PathBuf,

        /// Content type
        #[arg(long, default_value = "application/octet-stream")]
        content_type: String,
    },
}

#[derive(Subcommand, Debug)]
enum NodeCommands {
    /// List cluster nodes
    List,

    /// Get node info
    Info {
        /// Node ID (optional, defaults to connected node)
        id: Option<String>,
    },

    /// Health check
    Health,

    /// Get metrics
    Metrics,
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize colored output
    colored::control::set_override(true);

    match &args.command {
        Commands::Status => cmd_status(&args).await,
        Commands::Shell => cmd_shell(&args).await,
        Commands::Agents(cmd) => cmd_agents(&args, cmd).await,
        Commands::Services(cmd) => cmd_services(&args, cmd).await,
        Commands::Messages(cmd) => cmd_messages(&args, cmd).await,
        Commands::Nodes(cmd) => cmd_nodes(&args, cmd).await,
    }
}

// =============================================================================
// gRPC Client
// =============================================================================

async fn connect(node: &str, timeout: u64) -> Result<FipaAgentServiceClient<Channel>> {
    let endpoint = node.to_string();
    let channel = Channel::from_shared(endpoint)?
        .timeout(Duration::from_secs(timeout))
        .connect()
        .await
        .context("Failed to connect to node")?;

    Ok(FipaAgentServiceClient::new(channel))
}

// =============================================================================
// Command Handlers
// =============================================================================

async fn cmd_status(args: &Args) -> Result<()> {
    println!("{}", "FIPA Platform Status".bold().cyan());
    println!("{}", "─".repeat(40));

    let mut client = connect(&args.node, args.timeout).await?;

    // Health check
    let health = client
        .health_check(HealthCheckRequest { include_metrics: true })
        .await
        .context("Health check failed")?;

    let health = health.into_inner();
    let status = if health.healthy {
        "healthy".green()
    } else {
        "unhealthy".red()
    };
    println!("  {} {}", "Health:".bold(), status);

    // Node info
    let info = client
        .get_node_info(NodeInfoRequest {})
        .await
        .context("Failed to get node info")?;

    let info = info.into_inner();
    println!("  {} {}", "Node ID:".bold(), info.node_id);
    println!("  {} {:?}", "Addresses:".bold(), info.addresses);

    if let Some(caps) = info.capabilities {
        println!("  {} {} agents", "Capacity:".bold(), caps.max_agents);
    }

    if let Some(metrics) = info.metrics {
        println!("  {} {}", "Active Agents:".bold(), metrics.active_agents);
        println!("  {} {} sent, {} received", "Messages:".bold(), metrics.messages_sent, metrics.messages_received);
    }

    Ok(())
}

async fn cmd_shell(_args: &Args) -> Result<()> {
    println!("{}", "FIPA Interactive Shell".bold().cyan());
    println!("Type 'help' for commands, 'exit' to quit.\n");

    // Simple REPL
    let stdin = std::io::stdin();
    let mut input = String::new();

    loop {
        print!("{} ", "fipa>".green().bold());
        std::io::Write::flush(&mut std::io::stdout())?;

        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        match input {
            "" => continue,
            "exit" | "quit" | "q" => break,
            "help" | "?" => {
                println!("Available commands:");
                println!("  status     - Show platform status");
                println!("  agents     - List agents");
                println!("  services   - List services");
                println!("  health     - Health check");
                println!("  exit       - Exit shell");
            }
            "status" => {
                println!("Use: fipa-cli status");
            }
            "agents" => {
                println!("Use: fipa-cli agents list");
            }
            "services" => {
                println!("Use: fipa-cli services list");
            }
            "health" => {
                println!("Use: fipa-cli nodes health");
            }
            _ => {
                println!("{} Unknown command: {}", "Error:".red(), input);
                println!("Type 'help' for available commands.");
            }
        }
    }

    println!("Goodbye!");
    Ok(())
}

async fn cmd_agents(args: &Args, cmd: &AgentCommands) -> Result<()> {
    match cmd {
        AgentCommands::List => {
            println!("{}", "Agents".bold().cyan());
            println!("{}", "─".repeat(60));

            // For now, we query the node info for agent count
            // A full implementation would need a ListAgents RPC
            let mut client = connect(&args.node, args.timeout).await?;
            let info = client.get_node_info(NodeInfoRequest {}).await?;
            let info = info.into_inner();

            if let Some(metrics) = info.metrics {
                println!("  Active agents: {}", metrics.active_agents);
                println!("\n  {} Use 'fipa-cli agents status <name>' for details",
                    "Note:".yellow());
            } else {
                println!("  No agent information available");
            }

            Ok(())
        }

        AgentCommands::Create { name, wasm, owner } => {
            println!("{} Creating agent '{}'...", "→".cyan(), name);

            // Read WASM file
            let wasm_bytes = std::fs::read(&wasm)
                .with_context(|| format!("Failed to read WASM file: {:?}", wasm))?;

            println!("  Loaded {} bytes from {:?}", wasm_bytes.len(), wasm);

            // Note: Creating agents requires AMS integration
            // For now, we just validate the request
            println!("  Owner: {}", owner.as_deref().unwrap_or("(none)"));
            println!("\n  {} Agent creation via CLI requires AMS gRPC endpoint",
                "Note:".yellow());

            Ok(())
        }

        AgentCommands::Destroy { name } => {
            println!("{} Destroying agent '{}'...", "→".cyan(), name);
            println!("  {} Agent destruction via CLI requires AMS gRPC endpoint",
                "Note:".yellow());
            Ok(())
        }

        AgentCommands::Status { name } => {
            println!("{}", format!("Agent: {}", name).bold().cyan());
            println!("{}", "─".repeat(40));

            let mut client = connect(&args.node, args.timeout).await?;

            // Try to find the agent
            let response = client
                .find_agent(FindAgentRequest {
                    agent_id: Some(AgentId {
                        name: name.clone(),
                        addresses: vec![],
                        resolvers: vec![],
                    }),
                })
                .await;

            match response {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if resp.found {
                        println!("  {} {}", "Status:".bold(), "found".green());
                        if let Some(node_id) = &resp.node_id {
                            println!("  {} {}", "Node:".bold(), node_id);
                        }
                        if let Some(node_info) = &resp.node_info {
                            if let Some(caps) = &node_info.capabilities {
                                println!("  {} {} max agents", "Capacity:".bold(), caps.max_agents);
                            }
                        }
                    } else {
                        println!("  {} Agent not found", "Status:".bold());
                    }
                }
                Err(e) => {
                    println!("  {} {}", "Error:".red(), e);
                }
            }

            Ok(())
        }

        AgentCommands::Suspend { name } => {
            println!("{} Suspending agent '{}'...", "→".cyan(), name);
            println!("  {} Agent suspension via CLI requires AMS gRPC endpoint",
                "Note:".yellow());
            Ok(())
        }

        AgentCommands::Resume { name } => {
            println!("{} Resuming agent '{}'...", "→".cyan(), name);
            println!("  {} Agent resume via CLI requires AMS gRPC endpoint",
                "Note:".yellow());
            Ok(())
        }

        AgentCommands::Migrate { name, target } => {
            println!("{} Migrating agent '{}' to {}...", "→".cyan(), name, target);
            println!("  {} Migration via CLI coming soon", "Note:".yellow());
            Ok(())
        }

        AgentCommands::Clone { name, target, new_name } => {
            let clone_name = new_name.clone().unwrap_or_else(|| format!("{}-clone", name));
            println!("{} Cloning agent '{}' to {} as '{}'...",
                "→".cyan(), name, target, clone_name);
            println!("  {} Cloning via CLI coming soon", "Note:".yellow());
            Ok(())
        }
    }
}

async fn cmd_services(args: &Args, cmd: &ServiceCommands) -> Result<()> {
    match cmd {
        ServiceCommands::List => {
            println!("{}", "Registered Services".bold().cyan());
            println!("{}", "─".repeat(60));

            let mut client = connect(&args.node, args.timeout).await?;

            let response = client
                .find_service(FindServiceRequest {
                    service_name: String::new(), // Empty = list all
                    required_protocol: None,
                    ontology: None,
                    max_results: 100,
                })
                .await?;

            let response = response.into_inner();

            if response.providers.is_empty() {
                println!("  No services registered");
            } else {
                for provider in response.providers {
                    if let Some(agent_id) = provider.agent_id {
                        println!("  {} {}", "Provider:".bold(), agent_id.name);
                    }
                    if let Some(service) = provider.service {
                        println!("    {} {}", "Service:".bold(), service.name);
                        if !service.description.is_empty() {
                            println!("    {} {}", "Description:".bold(), service.description);
                        }
                    }
                    println!();
                }
            }

            Ok(())
        }

        ServiceCommands::Search { name, protocol, ontology } => {
            println!("{} '{}'", "Searching for service".bold().cyan(), name);
            println!("{}", "─".repeat(60));

            let mut client = connect(&args.node, args.timeout).await?;

            let proto_type = protocol.as_ref().map(|p| match p.to_lowercase().as_str() {
                "request" => ProtocolType::ProtocolRequest,
                "query" => ProtocolType::ProtocolQuery,
                "contract-net" => ProtocolType::ProtocolContractNet,
                "subscribe" => ProtocolType::ProtocolSubscribe,
                _ => ProtocolType::ProtocolUnspecified,
            });

            let response = client
                .find_service(FindServiceRequest {
                    service_name: name.clone(),
                    required_protocol: proto_type.map(|p| p as i32),
                    ontology: ontology.clone(),
                    max_results: 50,
                })
                .await?;

            let response = response.into_inner();

            if response.providers.is_empty() {
                println!("  No matching services found");
            } else {
                println!("  Found {} provider(s):", response.providers.len());
                for provider in response.providers {
                    if let Some(agent_id) = provider.agent_id {
                        println!("\n  {} {}", "Agent:".green().bold(), agent_id.name);
                        for addr in &agent_id.addresses {
                            println!("    Address: {}", addr);
                        }
                    }
                    if let Some(service) = provider.service {
                        println!("    Service: {}", service.name);
                    }
                }
            }

            Ok(())
        }

        ServiceCommands::Register { name, description, agent } => {
            println!("{} Registering service '{}' for agent '{}'...",
                "→".cyan(), name, agent);
            if let Some(desc) = description {
                println!("  Description: {}", desc);
            }
            println!("  {} Service registration via CLI requires DF gRPC endpoint",
                "Note:".yellow());
            Ok(())
        }

        ServiceCommands::Deregister { name, agent } => {
            println!("{} Deregistering service '{}' from agent '{}'...",
                "→".cyan(), name, agent);
            println!("  {} Service deregistration via CLI requires DF gRPC endpoint",
                "Note:".yellow());
            Ok(())
        }
    }
}

async fn cmd_messages(args: &Args, cmd: &MessageCommands) -> Result<()> {
    match cmd {
        MessageCommands::Send { to, content, performative, protocol, from } => {
            println!("{} Sending message to '{}'", "→".cyan(), to);
            println!("{}", "─".repeat(40));

            let mut client = connect(&args.node, args.timeout).await?;

            // Parse performative
            let perf = match performative.to_lowercase().as_str() {
                "request" => Performative::Request,
                "inform" => Performative::Inform,
                "query-ref" => Performative::QueryRef,
                "cfp" => Performative::Cfp,
                "propose" => Performative::Propose,
                "accept-proposal" => Performative::AcceptProposal,
                "reject-proposal" => Performative::RejectProposal,
                "agree" => Performative::Agree,
                "refuse" => Performative::Refuse,
                "cancel" => Performative::Cancel,
                _ => {
                    println!("  {} Unknown performative '{}', using 'request'",
                        "Warning:".yellow(), performative);
                    Performative::Request
                }
            };

            // Parse protocol
            let proto = match protocol.to_lowercase().as_str() {
                "request" => ProtocolType::ProtocolRequest,
                "query" => ProtocolType::ProtocolQuery,
                "contract-net" => ProtocolType::ProtocolContractNet,
                "subscribe" => ProtocolType::ProtocolSubscribe,
                "propose" => ProtocolType::ProtocolPropose,
                _ => ProtocolType::ProtocolRequest,
            };

            // Build message
            let message = AclMessage {
                message_id: format!("cli-{}", uuid::Uuid::new_v4()),
                performative: perf as i32,
                sender: Some(AgentId {
                    name: from.clone(),
                    addresses: vec![],
                    resolvers: vec![],
                }),
                receivers: vec![AgentId {
                    name: to.clone(),
                    addresses: vec![],
                    resolvers: vec![],
                }],
                reply_to: None,
                protocol: Some(proto as i32),
                conversation_id: Some(format!("conv-{}", uuid::Uuid::new_v4())),
                in_reply_to: None,
                reply_with: None,
                reply_by: None,
                language: Some("text/plain".to_string()),
                encoding: None,
                ontology: None,
                content: content.clone().into_bytes(),
                user_properties: std::collections::HashMap::new(),
            };

            println!("  From: {}", from);
            println!("  To: {}", to);
            println!("  Performative: {:?}", perf);
            println!("  Protocol: {:?}", proto);
            println!("  Content: {} bytes", message.content.len());

            let response = client.send_message(message).await?;
            let response = response.into_inner();

            if response.success {
                println!("\n  {} Message sent (ID: {})",
                    "✓".green().bold(),
                    response.message_id);
            } else {
                println!("\n  {} Failed: {}",
                    "✗".red().bold(),
                    response.error.unwrap_or_else(|| "unknown error".to_string()));
            }

            Ok(())
        }

        MessageCommands::Sniff { agent, duration } => {
            println!("{} Sniffing messages for agent '{}'", "→".cyan(), agent);
            if *duration > 0 {
                println!("  Duration: {} seconds", duration);
            } else {
                println!("  Duration: indefinite (Ctrl+C to stop)");
            }
            println!("{}", "─".repeat(40));

            // Note: This would use SubscribeMessages RPC
            println!("  {} Message sniffing via CLI coming soon", "Note:".yellow());

            Ok(())
        }

        MessageCommands::SendFile { to, file, content_type } => {
            println!("{} Sending file to '{}'", "→".cyan(), to);

            let content = std::fs::read(&file)
                .with_context(|| format!("Failed to read file: {:?}", file))?;

            println!("  File: {:?}", file);
            println!("  Size: {} bytes", content.len());
            println!("  Content-Type: {}", content_type);
            println!("  {} File sending via CLI coming soon", "Note:".yellow());

            Ok(())
        }
    }
}

async fn cmd_nodes(args: &Args, cmd: &NodeCommands) -> Result<()> {
    match cmd {
        NodeCommands::List => {
            println!("{}", "Cluster Nodes".bold().cyan());
            println!("{}", "─".repeat(60));

            let mut client = connect(&args.node, args.timeout).await?;
            let info = client.get_node_info(NodeInfoRequest {}).await?;
            let info = info.into_inner();

            println!("  {} {}", "Current Node:".bold(), info.node_id);
            for addr in &info.addresses {
                println!("    {}", addr);
            }

            println!("\n  {} Cluster discovery coming soon", "Note:".yellow());

            Ok(())
        }

        NodeCommands::Info { id } => {
            let node_id = id.clone().unwrap_or_else(|| "current".to_string());
            println!("{}", format!("Node: {}", node_id).bold().cyan());
            println!("{}", "─".repeat(40));

            let mut client = connect(&args.node, args.timeout).await?;
            let info = client.get_node_info(NodeInfoRequest {}).await?;
            let info = info.into_inner();

            println!("  {} {}", "Node ID:".bold(), info.node_id);
            println!("  {} {:?}", "Addresses:".bold(), info.addresses);

            if let Some(caps) = info.capabilities {
                println!("\n  {}", "Capabilities:".bold());
                println!("    Max agents: {}", caps.max_agents);
                println!("    Total memory: {} bytes", caps.total_memory);
                println!("    WASM runtime: {}", caps.wasm_runtime_version);
            }

            if let Some(metrics) = info.metrics {
                println!("\n  {}", "Metrics:".bold());
                println!("    Active agents: {}", metrics.active_agents);
                println!("    Messages: {} sent, {} received", metrics.messages_sent, metrics.messages_received);
                println!("    CPU usage: {:.1}%", metrics.cpu_usage_percent);
                println!("    Memory: {} used / {} available", metrics.memory_used_bytes, metrics.memory_available_bytes);
            }

            Ok(())
        }

        NodeCommands::Health => {
            println!("{}", "Health Check".bold().cyan());
            println!("{}", "─".repeat(40));

            let mut client = connect(&args.node, args.timeout).await?;
            let response = client.health_check(HealthCheckRequest { include_metrics: true }).await?;
            let response = response.into_inner();

            if response.healthy {
                println!("  {} Node is healthy", "✓".green().bold());
            } else {
                println!("  {} Node is unhealthy", "✗".red().bold());
            }

            println!("  Status: {}", response.status);

            if let Some(metrics) = response.metrics {
                println!("  Active agents: {}", metrics.active_agents);
                println!("  CPU: {:.1}%", metrics.cpu_usage_percent);
            }

            Ok(())
        }

        NodeCommands::Metrics => {
            println!("{}", "Node Metrics".bold().cyan());
            println!("{}", "─".repeat(40));

            println!("  Metrics endpoint: http://localhost:9090/metrics");
            println!("\n  {} Use 'curl {}' for Prometheus metrics",
                "Tip:".yellow(), "http://localhost:9090/metrics");

            Ok(())
        }
    }
}
