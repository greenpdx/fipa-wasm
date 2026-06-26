// observability/mod.rs - Tracing and Metrics

//! Observability module providing distributed tracing and Prometheus metrics.
//!
//! # Features
//!
//! - **Tracing**: Structured logging with span context propagation
//! - **Metrics**: Prometheus-compatible metrics export
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::observability::{init_tracing, init_metrics, MetricsConfig};
//!
//! // Initialize tracing
//! init_tracing(TracingConfig::default());
//!
//! // Initialize metrics
//! let handle = init_metrics(MetricsConfig::default()).unwrap();
//! ```

mod metrics;
mod tracing_setup;

pub use metrics::{
    init_metrics, record_agent_spawned, record_agent_stopped, record_message_sent,
    record_message_received, record_message_latency, record_consensus_commit,
    record_consensus_election, record_wasm_execution, record_migration, AgentMetrics,
    ConsensusMetrics, MessageMetrics, MetricsConfig, MetricsHandle,
};

pub use tracing_setup::{init_tracing, TracingConfig, TracingFormat};
