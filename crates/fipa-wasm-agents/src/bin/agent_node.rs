// bin/agent_node.rs - FIPA Agent Node Binary

use anyhow::Result;
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::Server;
use tracing::info;

use fipa_wasm_agents::consensus::{RaftNetwork, RaftStore};
use fipa_wasm_agents::network::grpc::{ConsensusServiceImpl, ConsensusState, StandaloneFipaService, StandaloneServiceConfig};
use fipa_wasm_agents::observability::{init_metrics, init_tracing, MetricsConfig, TracingConfig, TracingFormat};
use fipa_wasm_agents::proto;

/// FIPA Agent Node
#[derive(Parser, Debug)]
#[command(name = "fipa-node")]
#[command(author = "SavageS")]
#[command(version)]
#[command(about = "FIPA WASM Distributed Agent Node", long_about = None)]
struct Args {
    /// Node ID (numeric)
    #[arg(short, long, default_value = "1")]
    node_id: u64,

    /// Node name
    #[arg(long, default_value = "node-1")]
    name: String,

    /// gRPC listen address
    #[arg(short, long, default_value = "0.0.0.0:9000")]
    listen: String,

    /// Data directory
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,

    /// Config file path
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Log format (pretty, compact, json)
    #[arg(long, default_value = "pretty")]
    log_format: String,

    /// Enable metrics server
    #[arg(long)]
    metrics: bool,

    /// Metrics listen address
    #[arg(long, default_value = "0.0.0.0:9090")]
    metrics_addr: String,

    /// Bootstrap peer addresses (can be specified multiple times)
    #[arg(long)]
    bootstrap: Vec<String>,

    /// Enable Raft consensus
    #[arg(long)]
    consensus: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let tracing_config = TracingConfig {
        filter: format!("{},fipa_wasm_agents={}", args.log_level, args.log_level),
        format: match args.log_format.as_str() {
            "json" => TracingFormat::Json,
            "compact" => TracingFormat::Compact,
            _ => TracingFormat::Pretty,
        },
        with_span_events: args.log_level == "trace" || args.log_level == "debug",
        with_file: args.log_level == "debug" || args.log_level == "trace",
        with_target: true,
        with_thread_ids: args.log_level == "trace",
        with_thread_names: false,
        with_ansi: args.log_format != "json",
    };
    init_tracing(tracing_config);

    info!("Starting FIPA Agent Node");
    info!(node_id = args.node_id, name = %args.name, "Node identity");
    info!(listen = %args.listen, "gRPC endpoint");

    // Create data directory if needed
    if !args.data_dir.exists() {
        std::fs::create_dir_all(&args.data_dir)?;
        info!(path = ?args.data_dir, "Created data directory");
    }

    // Initialize metrics if enabled
    let _metrics_handle = if args.metrics {
        let metrics_addr: SocketAddr = args.metrics_addr.parse()?;
        let metrics_config = MetricsConfig {
            listen_addr: metrics_addr,
            ..Default::default()
        };

        match init_metrics(metrics_config) {
            Ok(handle) => {
                info!(addr = %metrics_addr, "Metrics recorder initialized");
                Some(handle)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize metrics");
                None
            }
        }
    } else {
        None
    };

    // Initialize Raft consensus storage if enabled
    let _raft_store = if args.consensus {
        let raft_path = args.data_dir.join("raft");
        match RaftStore::open(&raft_path, args.node_id) {
            Ok(store) => {
                info!(path = ?raft_path, "Raft storage initialized");
                Some(Arc::new(store))
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to open Raft storage");
                return Err(e.into());
            }
        }
    } else {
        None
    };

    // Initialize Raft network
    let _raft_network = if args.consensus {
        let network = Arc::new(RaftNetwork::new());

        // Add self
        network.add_node(args.node_id, fipa_wasm_agents::consensus::NodeInfo {
            grpc_addr: args.listen.clone(),
            peer_id: None,
            name: Some(args.name.clone()),
        });

        // Add bootstrap peers
        for (i, peer) in args.bootstrap.iter().enumerate() {
            let peer_id = (i + 2) as u64; // Start from 2, assuming we are 1
            network.add_node(peer_id, fipa_wasm_agents::consensus::NodeInfo {
                grpc_addr: peer.clone(),
                peer_id: None,
                name: Some(format!("node-{}", peer_id)),
            });
            info!(peer_id, addr = %peer, "Added bootstrap peer");
        }

        Some(network)
    } else {
        None
    };

    // Parse gRPC listen address
    let grpc_addr: SocketAddr = args.listen.parse()?;

    // Create consensus state for gRPC
    let consensus_state = Arc::new(RwLock::new(ConsensusState::default()));

    info!("Node {} ({}) starting gRPC server on {}", args.node_id, args.name, grpc_addr);
    info!("Press Ctrl+C to shutdown");

    // Start gRPC server with services and reflection
    let consensus_service = ConsensusServiceImpl::with_state(consensus_state);

    // Create standalone FIPA agent service
    let fipa_service = StandaloneFipaService::new(StandaloneServiceConfig {
        node_id: args.node_id.to_string(),
        node_name: args.name.clone(),
        grpc_addr: args.listen.clone(),
    });

    // Load file descriptor for reflection
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(fipa_wasm_agents::proto::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let grpc_server = Server::builder()
        .add_service(reflection_service)
        .add_service(proto::fipa_agent_service_server::FipaAgentServiceServer::new(fipa_service))
        .add_service(proto::consensus_service_server::ConsensusServiceServer::new(consensus_service))
        .serve(grpc_addr);

    // Wait for shutdown signal
    tokio::select! {
        result = grpc_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "gRPC server error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("Shutting down...");
    Ok(())
}
