// observability/metrics.rs - Prometheus Metrics

use metrics::{counter, gauge, histogram, describe_counter, describe_gauge, describe_histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use std::time::Duration;

/// Configuration for metrics
#[derive(Clone, Debug)]
pub struct MetricsConfig {
    /// Address to expose metrics endpoint
    pub listen_addr: SocketAddr,

    /// Histogram buckets for latency metrics (in seconds)
    pub latency_buckets: Vec<f64>,

    /// Histogram buckets for execution time (in seconds)
    pub execution_buckets: Vec<f64>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9090".parse().unwrap(),
            latency_buckets: vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
            execution_buckets: vec![
                0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0,
            ],
        }
    }
}

/// Handle to the Prometheus metrics exporter
#[derive(Clone)]
pub struct MetricsHandle {
    handle: PrometheusHandle,
}

impl MetricsHandle {
    /// Render metrics in Prometheus text format
    pub fn render(&self) -> String {
        self.handle.render()
    }
}

/// Agent-related metrics
pub struct AgentMetrics;

impl AgentMetrics {
    pub const SPAWNED_TOTAL: &'static str = "fipa_agents_spawned_total";
    pub const STOPPED_TOTAL: &'static str = "fipa_agents_stopped_total";
    pub const ACTIVE: &'static str = "fipa_agents_active";
    pub const MIGRATIONS_TOTAL: &'static str = "fipa_agent_migrations_total";
}

/// Message-related metrics
pub struct MessageMetrics;

impl MessageMetrics {
    pub const SENT_TOTAL: &'static str = "fipa_messages_sent_total";
    pub const RECEIVED_TOTAL: &'static str = "fipa_messages_received_total";
    pub const LATENCY_SECONDS: &'static str = "fipa_message_latency_seconds";
}

/// Consensus-related metrics
pub struct ConsensusMetrics;

impl ConsensusMetrics {
    pub const COMMITS_TOTAL: &'static str = "fipa_consensus_commits_total";
    pub const ELECTIONS_TOTAL: &'static str = "fipa_consensus_elections_total";
    pub const LEADER_CHANGES_TOTAL: &'static str = "fipa_consensus_leader_changes_total";
    pub const TERM: &'static str = "fipa_consensus_term";
}

/// WASM execution metrics
pub struct WasmMetrics;

impl WasmMetrics {
    pub const EXECUTION_SECONDS: &'static str = "fipa_wasm_execution_seconds";
    pub const FUEL_CONSUMED: &'static str = "fipa_wasm_fuel_consumed";
}

/// Initialize the metrics system
///
/// Starts an HTTP server on the configured address to expose Prometheus metrics.
/// Returns a handle that can be used to render metrics programmatically.
pub fn init_metrics(config: MetricsConfig) -> Result<MetricsHandle, Box<dyn std::error::Error>> {
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(MessageMetrics::LATENCY_SECONDS.into()),
            &config.latency_buckets,
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(WasmMetrics::EXECUTION_SECONDS.into()),
            &config.execution_buckets,
        )?;

    let handle = builder.install_recorder()?;
    let metrics_handle = MetricsHandle { handle: handle.clone() };

    // Start HTTP server for metrics endpoint
    let listen_addr = config.listen_addr;
    let shared_handle = std::sync::Arc::new(handle);

    tokio::spawn(async move {
        use axum::{routing::get, Router, Json, http::StatusCode};
        use serde::Serialize;

        #[derive(Serialize)]
        struct HealthResponse {
            status: &'static str,
            version: &'static str,
            uptime_secs: u64,
        }

        let start_time = std::time::Instant::now();

        let handle_for_route = shared_handle.clone();
        let app = Router::new()
            .route("/metrics", get(move || {
                let h = handle_for_route.clone();
                async move { h.render() }
            }))
            .route("/health", get(move || {
                let uptime = start_time.elapsed().as_secs();
                async move {
                    Json(HealthResponse {
                        status: "healthy",
                        version: env!("CARGO_PKG_VERSION"),
                        uptime_secs: uptime,
                    })
                }
            }))
            .route("/ready", get(|| async { StatusCode::OK }))
            .route("/live", get(|| async { StatusCode::OK }));

        match tokio::net::TcpListener::bind(listen_addr).await {
            Ok(listener) => {
                tracing::info!(addr = %listen_addr, "Metrics HTTP server started");
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::error!(error = %e, "Metrics server error");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, addr = %listen_addr, "Failed to bind metrics server");
            }
        }
    });

    // Register metric descriptions
    describe_counter!(
        AgentMetrics::SPAWNED_TOTAL,
        "Total number of agents spawned"
    );
    describe_counter!(
        AgentMetrics::STOPPED_TOTAL,
        "Total number of agents stopped"
    );
    describe_gauge!(
        AgentMetrics::ACTIVE,
        "Current number of active agents"
    );
    describe_counter!(
        AgentMetrics::MIGRATIONS_TOTAL,
        "Total number of agent migrations"
    );

    describe_counter!(
        MessageMetrics::SENT_TOTAL,
        "Total number of messages sent"
    );
    describe_counter!(
        MessageMetrics::RECEIVED_TOTAL,
        "Total number of messages received"
    );
    describe_histogram!(
        MessageMetrics::LATENCY_SECONDS,
        "Message delivery latency in seconds"
    );

    describe_counter!(
        ConsensusMetrics::COMMITS_TOTAL,
        "Total number of Raft commits"
    );
    describe_counter!(
        ConsensusMetrics::ELECTIONS_TOTAL,
        "Total number of leader elections"
    );
    describe_counter!(
        ConsensusMetrics::LEADER_CHANGES_TOTAL,
        "Total number of leader changes"
    );
    describe_gauge!(
        ConsensusMetrics::TERM,
        "Current Raft term"
    );

    describe_histogram!(
        WasmMetrics::EXECUTION_SECONDS,
        "WASM function execution time in seconds"
    );
    describe_counter!(
        WasmMetrics::FUEL_CONSUMED,
        "Total fuel consumed by WASM execution"
    );

    tracing::info!(addr = %config.listen_addr, "Metrics initialized");

    // Record initial metrics to ensure something is always visible
    counter!(AgentMetrics::SPAWNED_TOTAL, "agent_type" => "system").increment(1);
    gauge!(AgentMetrics::ACTIVE, "agent_type" => "system").set(1.0);

    Ok(metrics_handle)
}

// Recording functions

/// Record an agent being spawned
pub fn record_agent_spawned(agent_type: &str) {
    counter!(AgentMetrics::SPAWNED_TOTAL, "agent_type" => agent_type.to_string()).increment(1);
    gauge!(AgentMetrics::ACTIVE, "agent_type" => agent_type.to_string()).increment(1.0);
}

/// Record an agent being stopped
pub fn record_agent_stopped(agent_type: &str, reason: &str) {
    counter!(
        AgentMetrics::STOPPED_TOTAL,
        "agent_type" => agent_type.to_string(),
        "reason" => reason.to_string()
    ).increment(1);
    gauge!(AgentMetrics::ACTIVE, "agent_type" => agent_type.to_string()).decrement(1.0);
}

/// Record a message being sent
pub fn record_message_sent(performative: &str, protocol: &str) {
    counter!(
        MessageMetrics::SENT_TOTAL,
        "performative" => performative.to_string(),
        "protocol" => protocol.to_string()
    ).increment(1);
}

/// Record a message being received
pub fn record_message_received(performative: &str, protocol: &str) {
    counter!(
        MessageMetrics::RECEIVED_TOTAL,
        "performative" => performative.to_string(),
        "protocol" => protocol.to_string()
    ).increment(1);
}

/// Record message latency
pub fn record_message_latency(latency: Duration, performative: &str) {
    histogram!(
        MessageMetrics::LATENCY_SECONDS,
        "performative" => performative.to_string()
    ).record(latency.as_secs_f64());
}

/// Record a consensus commit
pub fn record_consensus_commit(log_index: u64) {
    counter!(ConsensusMetrics::COMMITS_TOTAL).increment(1);
    tracing::trace!(log_index, "Consensus commit recorded");
}

/// Record a leader election
pub fn record_consensus_election(term: u64, became_leader: bool) {
    counter!(
        ConsensusMetrics::ELECTIONS_TOTAL,
        "became_leader" => became_leader.to_string()
    ).increment(1);
    gauge!(ConsensusMetrics::TERM).set(term as f64);
}

/// Record WASM execution
pub fn record_wasm_execution(duration: Duration, function: &str, fuel: Option<u64>) {
    histogram!(
        WasmMetrics::EXECUTION_SECONDS,
        "function" => function.to_string()
    ).record(duration.as_secs_f64());

    if let Some(fuel) = fuel {
        counter!(
            WasmMetrics::FUEL_CONSUMED,
            "function" => function.to_string()
        ).increment(fuel);
    }
}

/// Record an agent migration
pub fn record_migration(from_node: &str, to_node: &str, success: bool) {
    counter!(
        AgentMetrics::MIGRATIONS_TOTAL,
        "from_node" => from_node.to_string(),
        "to_node" => to_node.to_string(),
        "success" => success.to_string()
    ).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_config_default() {
        let config = MetricsConfig::default();
        assert!(!config.latency_buckets.is_empty());
        assert!(!config.execution_buckets.is_empty());
    }

    #[test]
    fn test_metric_names() {
        assert!(AgentMetrics::SPAWNED_TOTAL.starts_with("fipa_"));
        assert!(MessageMetrics::SENT_TOTAL.starts_with("fipa_"));
        assert!(ConsensusMetrics::COMMITS_TOTAL.starts_with("fipa_"));
    }
}
