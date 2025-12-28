// tools/dashboard.rs - FIPA Web Dashboard
//
//! Web-based Dashboard for FIPA Agent Platforms
//!
//! Provides a browser-based interface for:
//! - Agent listing and status monitoring
//! - Service discovery and browsing
//! - Message inspection
//! - Platform metrics visualization
//! - Node topology view
//!
//! # Usage
//!
//! ```ignore
//! use fipa_wasm_agents::tools::dashboard::{Dashboard, DashboardConfig};
//!
//! let config = DashboardConfig::default();
//! let dashboard = Dashboard::new(config);
//! dashboard.run().await?;
//! ```

use axum::{
    extract::{Path, State, Query},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Dashboard configuration
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    /// HTTP listen address
    pub listen_addr: SocketAddr,

    /// gRPC node address for data
    pub grpc_addr: String,

    /// Refresh interval in milliseconds
    pub refresh_interval_ms: u64,

    /// Enable CORS
    pub enable_cors: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9091".parse().unwrap(),
            grpc_addr: "http://localhost:9000".to_string(),
            refresh_interval_ms: 2000,
            enable_cors: true,
        }
    }
}

/// Agent info for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub status: String,
    pub node_id: String,
    pub services: Vec<String>,
    pub message_count: u64,
    pub created_at: String,
}

/// Service info for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub description: String,
    pub provider: String,
    pub protocol: String,
    pub ontology: String,
}

/// Node info for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    pub addresses: Vec<String>,
    pub agent_count: u32,
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub healthy: bool,
}

/// Platform metrics for dashboard
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformMetrics {
    pub total_agents: u32,
    pub total_services: u32,
    pub total_nodes: u32,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub uptime_seconds: u64,
}

/// Message info for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    pub timestamp: String,
    pub sender: String,
    pub receiver: String,
    pub performative: String,
    pub protocol: String,
    pub conversation_id: String,
    pub content_preview: String,
}

/// Dashboard state
#[derive(Debug, Default)]
pub struct DashboardState {
    pub agents: Vec<AgentInfo>,
    pub services: Vec<ServiceInfo>,
    pub nodes: Vec<NodeInfo>,
    pub metrics: PlatformMetrics,
    pub messages: Vec<MessageInfo>,
    pub last_update: Option<std::time::Instant>,
}

/// Shared dashboard state
pub type SharedState = Arc<RwLock<DashboardState>>;

/// Dashboard server
pub struct Dashboard {
    config: DashboardConfig,
    state: SharedState,
}

impl Dashboard {
    /// Create a new dashboard
    pub fn new(config: DashboardConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(DashboardState::default())),
        }
    }

    /// Run the dashboard server
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let addr = self.config.listen_addr;
        let state = self.state.clone();

        // Build router
        let app = Router::new()
            // HTML pages
            .route("/", get(serve_index))
            .route("/dashboard", get(serve_index))
            // Static assets
            .route("/static/style.css", get(serve_css))
            .route("/static/app.js", get(serve_js))
            // API endpoints
            .route("/api/agents", get(api_get_agents))
            .route("/api/agents/:name", get(api_get_agent))
            .route("/api/services", get(api_get_services))
            .route("/api/nodes", get(api_get_nodes))
            .route("/api/metrics", get(api_get_metrics))
            .route("/api/messages", get(api_get_messages))
            .route("/api/health", get(api_health))
            // Actions
            .route("/api/agents/:name/suspend", post(api_suspend_agent))
            .route("/api/agents/:name/resume", post(api_resume_agent))
            .with_state(state);

        info!("Dashboard starting on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }

    /// Get the shared state
    pub fn state(&self) -> SharedState {
        self.state.clone()
    }
}

// =============================================================================
// HTML/Static Content
// =============================================================================

async fn serve_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn serve_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        STYLE_CSS,
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        APP_JS,
    )
}

// =============================================================================
// API Handlers
// =============================================================================

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn api_get_agents(
    State(state): State<SharedState>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<AgentInfo>> {
    let state = state.read().await;
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let agents: Vec<_> = state.agents
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect();

    Json(agents)
}

async fn api_get_agent(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<AgentInfo>, StatusCode> {
    let state = state.read().await;

    state.agents
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn api_get_services(
    State(state): State<SharedState>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<ServiceInfo>> {
    let state = state.read().await;
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let services: Vec<_> = state.services
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect();

    Json(services)
}

async fn api_get_nodes(
    State(state): State<SharedState>,
) -> Json<Vec<NodeInfo>> {
    let state = state.read().await;
    Json(state.nodes.clone())
}

async fn api_get_metrics(
    State(state): State<SharedState>,
) -> Json<PlatformMetrics> {
    let state = state.read().await;
    Json(state.metrics.clone())
}

async fn api_get_messages(
    State(state): State<SharedState>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<MessageInfo>> {
    let state = state.read().await;
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);

    let messages: Vec<_> = state.messages
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect();

    Json(messages)
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    dashboard_version: String,
    agents_loaded: usize,
    services_loaded: usize,
}

async fn api_health(
    State(state): State<SharedState>,
) -> Json<HealthResponse> {
    let state = state.read().await;

    Json(HealthResponse {
        status: "ok".to_string(),
        dashboard_version: env!("CARGO_PKG_VERSION").to_string(),
        agents_loaded: state.agents.len(),
        services_loaded: state.services.len(),
    })
}

#[derive(Serialize)]
struct ActionResponse {
    success: bool,
    message: String,
}

async fn api_suspend_agent(
    Path(name): Path<String>,
) -> Json<ActionResponse> {
    // In a real implementation, this would call the AMS
    warn!("Suspend agent '{}' - not yet implemented", name);

    Json(ActionResponse {
        success: false,
        message: "Agent suspension requires AMS gRPC integration".to_string(),
    })
}

async fn api_resume_agent(
    Path(name): Path<String>,
) -> Json<ActionResponse> {
    // In a real implementation, this would call the AMS
    warn!("Resume agent '{}' - not yet implemented", name);

    Json(ActionResponse {
        success: false,
        message: "Agent resume requires AMS gRPC integration".to_string(),
    })
}

// =============================================================================
// Embedded Static Content
// =============================================================================

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>FIPA Agent Platform Dashboard</title>
    <link rel="stylesheet" href="/static/style.css">
</head>
<body>
    <header>
        <h1>FIPA Agent Platform</h1>
        <div id="status">
            <span id="connection-status" class="status-indicator"></span>
            <span id="status-text">Connecting...</span>
        </div>
    </header>

    <nav>
        <button class="nav-btn active" data-tab="agents">Agents</button>
        <button class="nav-btn" data-tab="services">Services</button>
        <button class="nav-btn" data-tab="messages">Messages</button>
        <button class="nav-btn" data-tab="nodes">Nodes</button>
        <button class="nav-btn" data-tab="metrics">Metrics</button>
    </nav>

    <main>
        <!-- Agents Tab -->
        <section id="agents-tab" class="tab-content active">
            <div class="section-header">
                <h2>Agents</h2>
                <button id="refresh-agents" class="btn">Refresh</button>
            </div>
            <table id="agents-table">
                <thead>
                    <tr>
                        <th>Name</th>
                        <th>Status</th>
                        <th>Node</th>
                        <th>Services</th>
                        <th>Messages</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="agents-body">
                    <tr><td colspan="6" class="loading">Loading agents...</td></tr>
                </tbody>
            </table>
        </section>

        <!-- Services Tab -->
        <section id="services-tab" class="tab-content">
            <div class="section-header">
                <h2>Services</h2>
                <input type="text" id="service-search" placeholder="Search services...">
            </div>
            <table id="services-table">
                <thead>
                    <tr>
                        <th>Name</th>
                        <th>Description</th>
                        <th>Provider</th>
                        <th>Protocol</th>
                        <th>Ontology</th>
                    </tr>
                </thead>
                <tbody id="services-body">
                    <tr><td colspan="5" class="loading">Loading services...</td></tr>
                </tbody>
            </table>
        </section>

        <!-- Messages Tab -->
        <section id="messages-tab" class="tab-content">
            <div class="section-header">
                <h2>Message Stream</h2>
                <div>
                    <label><input type="checkbox" id="auto-scroll" checked> Auto-scroll</label>
                    <button id="clear-messages" class="btn">Clear</button>
                </div>
            </div>
            <div id="message-stream">
                <div class="no-messages">No messages captured yet</div>
            </div>
        </section>

        <!-- Nodes Tab -->
        <section id="nodes-tab" class="tab-content">
            <div class="section-header">
                <h2>Cluster Nodes</h2>
            </div>
            <div id="nodes-grid">
                <div class="loading">Loading nodes...</div>
            </div>
        </section>

        <!-- Metrics Tab -->
        <section id="metrics-tab" class="tab-content">
            <div class="section-header">
                <h2>Platform Metrics</h2>
            </div>
            <div id="metrics-grid">
                <div class="metric-card">
                    <div class="metric-label">Total Agents</div>
                    <div class="metric-value" id="metric-agents">-</div>
                </div>
                <div class="metric-card">
                    <div class="metric-label">Total Services</div>
                    <div class="metric-value" id="metric-services">-</div>
                </div>
                <div class="metric-card">
                    <div class="metric-label">Cluster Nodes</div>
                    <div class="metric-value" id="metric-nodes">-</div>
                </div>
                <div class="metric-card">
                    <div class="metric-label">Messages Sent</div>
                    <div class="metric-value" id="metric-sent">-</div>
                </div>
                <div class="metric-card">
                    <div class="metric-label">Messages Received</div>
                    <div class="metric-value" id="metric-received">-</div>
                </div>
                <div class="metric-card">
                    <div class="metric-label">Uptime</div>
                    <div class="metric-value" id="metric-uptime">-</div>
                </div>
            </div>
        </section>
    </main>

    <footer>
        <p>FIPA WASM Agent Platform &copy; 2024 | <span id="version">v0.2.0</span></p>
    </footer>

    <script src="/static/app.js"></script>
</body>
</html>
"#;

const STYLE_CSS: &str = r#"
:root {
    --bg-primary: #1a1a2e;
    --bg-secondary: #16213e;
    --bg-tertiary: #0f3460;
    --text-primary: #eaeaea;
    --text-secondary: #a0a0a0;
    --accent: #e94560;
    --success: #4ecca3;
    --warning: #ffc107;
    --error: #e94560;
    --border: #2a2a4a;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
    background: var(--bg-primary);
    color: var(--text-primary);
    min-height: 100vh;
    display: flex;
    flex-direction: column;
}

header {
    background: var(--bg-secondary);
    padding: 1rem 2rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--border);
}

header h1 {
    font-size: 1.5rem;
    color: var(--accent);
}

#status {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

.status-indicator {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--warning);
}

.status-indicator.connected { background: var(--success); }
.status-indicator.disconnected { background: var(--error); }

nav {
    background: var(--bg-secondary);
    padding: 0.5rem 2rem;
    display: flex;
    gap: 0.5rem;
    border-bottom: 1px solid var(--border);
}

.nav-btn {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 0.75rem 1.5rem;
    cursor: pointer;
    border-radius: 4px;
    transition: all 0.2s;
    font-size: 0.9rem;
}

.nav-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
}

.nav-btn.active {
    background: var(--bg-tertiary);
    color: var(--accent);
}

main {
    flex: 1;
    padding: 2rem;
    overflow-y: auto;
}

.tab-content {
    display: none;
}

.tab-content.active {
    display: block;
}

.section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
}

.section-header h2 {
    font-size: 1.25rem;
    color: var(--text-primary);
}

.btn {
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.5rem 1rem;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s;
}

.btn:hover {
    background: var(--accent);
    border-color: var(--accent);
}

table {
    width: 100%;
    border-collapse: collapse;
    background: var(--bg-secondary);
    border-radius: 8px;
    overflow: hidden;
}

th, td {
    padding: 1rem;
    text-align: left;
    border-bottom: 1px solid var(--border);
}

th {
    background: var(--bg-tertiary);
    color: var(--text-secondary);
    font-weight: 500;
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.5px;
}

tr:hover {
    background: var(--bg-tertiary);
}

.loading {
    text-align: center;
    color: var(--text-secondary);
    padding: 2rem;
}

.status-badge {
    padding: 0.25rem 0.75rem;
    border-radius: 12px;
    font-size: 0.75rem;
    text-transform: uppercase;
}

.status-badge.running { background: var(--success); color: #000; }
.status-badge.suspended { background: var(--warning); color: #000; }
.status-badge.stopped { background: var(--error); color: #fff; }

.action-btn {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    cursor: pointer;
    margin-right: 0.25rem;
    font-size: 0.75rem;
}

.action-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
}

#service-search {
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.5rem 1rem;
    border-radius: 4px;
    width: 250px;
}

#message-stream {
    background: var(--bg-secondary);
    border-radius: 8px;
    padding: 1rem;
    height: 500px;
    overflow-y: auto;
    font-family: monospace;
    font-size: 0.85rem;
}

.message-entry {
    padding: 0.5rem;
    border-bottom: 1px solid var(--border);
}

.message-entry:last-child {
    border-bottom: none;
}

.message-time {
    color: var(--text-secondary);
}

.message-sender {
    color: #4fc3f7;
}

.message-receiver {
    color: var(--success);
}

.message-performative {
    color: var(--warning);
}

.no-messages {
    color: var(--text-secondary);
    text-align: center;
    padding: 2rem;
}

#nodes-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 1rem;
}

.node-card {
    background: var(--bg-secondary);
    border-radius: 8px;
    padding: 1.5rem;
    border: 1px solid var(--border);
}

.node-card h3 {
    color: var(--accent);
    margin-bottom: 1rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.node-status {
    width: 10px;
    height: 10px;
    border-radius: 50%;
}

.node-status.healthy { background: var(--success); }
.node-status.unhealthy { background: var(--error); }

.node-stat {
    display: flex;
    justify-content: space-between;
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--border);
}

.node-stat:last-child {
    border-bottom: none;
}

.node-stat-label {
    color: var(--text-secondary);
}

#metrics-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 1rem;
}

.metric-card {
    background: var(--bg-secondary);
    border-radius: 8px;
    padding: 1.5rem;
    text-align: center;
    border: 1px solid var(--border);
}

.metric-label {
    color: var(--text-secondary);
    font-size: 0.85rem;
    margin-bottom: 0.5rem;
}

.metric-value {
    color: var(--accent);
    font-size: 2rem;
    font-weight: bold;
}

footer {
    background: var(--bg-secondary);
    padding: 1rem 2rem;
    text-align: center;
    color: var(--text-secondary);
    font-size: 0.85rem;
    border-top: 1px solid var(--border);
}

@media (max-width: 768px) {
    header {
        flex-direction: column;
        gap: 1rem;
    }

    nav {
        flex-wrap: wrap;
        justify-content: center;
    }

    .section-header {
        flex-direction: column;
        gap: 1rem;
    }
}
"#;

const APP_JS: &str = r#"
// FIPA Dashboard Application
(function() {
    'use strict';

    const API_BASE = '';
    let refreshInterval = 5000;
    let currentTab = 'agents';

    // Initialize
    document.addEventListener('DOMContentLoaded', init);

    function init() {
        setupNavigation();
        setupEventListeners();
        loadAllData();
        startAutoRefresh();
        updateConnectionStatus(true);
    }

    function setupNavigation() {
        document.querySelectorAll('.nav-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                const tab = btn.dataset.tab;
                switchTab(tab);
            });
        });
    }

    function switchTab(tab) {
        currentTab = tab;

        document.querySelectorAll('.nav-btn').forEach(btn => {
            btn.classList.toggle('active', btn.dataset.tab === tab);
        });

        document.querySelectorAll('.tab-content').forEach(content => {
            content.classList.toggle('active', content.id === tab + '-tab');
        });
    }

    function setupEventListeners() {
        document.getElementById('refresh-agents')?.addEventListener('click', loadAgents);
        document.getElementById('clear-messages')?.addEventListener('click', clearMessages);
        document.getElementById('service-search')?.addEventListener('input', filterServices);
    }

    async function loadAllData() {
        await Promise.all([
            loadAgents(),
            loadServices(),
            loadNodes(),
            loadMetrics(),
            loadMessages()
        ]);
    }

    function startAutoRefresh() {
        setInterval(loadAllData, refreshInterval);
    }

    async function fetchApi(endpoint) {
        try {
            const response = await fetch(API_BASE + endpoint);
            if (!response.ok) throw new Error(`HTTP ${response.status}`);
            return await response.json();
        } catch (error) {
            console.error(`API error for ${endpoint}:`, error);
            updateConnectionStatus(false);
            return null;
        }
    }

    async function loadAgents() {
        const agents = await fetchApi('/api/agents');
        if (!agents) return;

        const tbody = document.getElementById('agents-body');
        if (agents.length === 0) {
            tbody.innerHTML = '<tr><td colspan="6" class="loading">No agents registered</td></tr>';
            return;
        }

        tbody.innerHTML = agents.map(agent => `
            <tr>
                <td><strong>${escapeHtml(agent.name)}</strong></td>
                <td><span class="status-badge ${agent.status}">${agent.status}</span></td>
                <td>${escapeHtml(agent.node_id)}</td>
                <td>${agent.services.length}</td>
                <td>${agent.message_count}</td>
                <td>
                    <button class="action-btn" onclick="suspendAgent('${agent.name}')">Suspend</button>
                    <button class="action-btn" onclick="viewAgent('${agent.name}')">View</button>
                </td>
            </tr>
        `).join('');

        updateConnectionStatus(true);
    }

    async function loadServices() {
        const services = await fetchApi('/api/services');
        if (!services) return;

        window.allServices = services;
        renderServices(services);
    }

    function renderServices(services) {
        const tbody = document.getElementById('services-body');
        if (services.length === 0) {
            tbody.innerHTML = '<tr><td colspan="5" class="loading">No services registered</td></tr>';
            return;
        }

        tbody.innerHTML = services.map(service => `
            <tr>
                <td><strong>${escapeHtml(service.name)}</strong></td>
                <td>${escapeHtml(service.description)}</td>
                <td>${escapeHtml(service.provider)}</td>
                <td>${escapeHtml(service.protocol)}</td>
                <td>${escapeHtml(service.ontology)}</td>
            </tr>
        `).join('');
    }

    function filterServices(e) {
        const query = e.target.value.toLowerCase();
        const filtered = (window.allServices || []).filter(s =>
            s.name.toLowerCase().includes(query) ||
            s.description.toLowerCase().includes(query) ||
            s.provider.toLowerCase().includes(query)
        );
        renderServices(filtered);
    }

    async function loadNodes() {
        const nodes = await fetchApi('/api/nodes');
        if (!nodes) return;

        const grid = document.getElementById('nodes-grid');
        if (nodes.length === 0) {
            grid.innerHTML = '<div class="loading">No nodes in cluster</div>';
            return;
        }

        grid.innerHTML = nodes.map(node => `
            <div class="node-card">
                <h3>
                    ${escapeHtml(node.id)}
                    <span class="node-status ${node.healthy ? 'healthy' : 'unhealthy'}"></span>
                </h3>
                <div class="node-stat">
                    <span class="node-stat-label">Agents</span>
                    <span>${node.agent_count}</span>
                </div>
                <div class="node-stat">
                    <span class="node-stat-label">CPU Usage</span>
                    <span>${node.cpu_usage.toFixed(1)}%</span>
                </div>
                <div class="node-stat">
                    <span class="node-stat-label">Memory</span>
                    <span>${formatBytes(node.memory_used)} / ${formatBytes(node.memory_total)}</span>
                </div>
                <div class="node-stat">
                    <span class="node-stat-label">Addresses</span>
                    <span>${node.addresses.length}</span>
                </div>
            </div>
        `).join('');
    }

    async function loadMetrics() {
        const metrics = await fetchApi('/api/metrics');
        if (!metrics) return;

        document.getElementById('metric-agents').textContent = metrics.total_agents;
        document.getElementById('metric-services').textContent = metrics.total_services;
        document.getElementById('metric-nodes').textContent = metrics.total_nodes;
        document.getElementById('metric-sent').textContent = formatNumber(metrics.messages_sent);
        document.getElementById('metric-received').textContent = formatNumber(metrics.messages_received);
        document.getElementById('metric-uptime').textContent = formatUptime(metrics.uptime_seconds);
    }

    async function loadMessages() {
        const messages = await fetchApi('/api/messages');
        if (!messages) return;

        const stream = document.getElementById('message-stream');
        if (messages.length === 0) {
            stream.innerHTML = '<div class="no-messages">No messages captured yet</div>';
            return;
        }

        stream.innerHTML = messages.map(msg => `
            <div class="message-entry">
                <span class="message-time">${msg.timestamp}</span>
                <span class="message-sender">${escapeHtml(msg.sender)}</span>
                <span>-&gt;</span>
                <span class="message-receiver">${escapeHtml(msg.receiver)}</span>
                <span class="message-performative">[${msg.performative}]</span>
                <span>${escapeHtml(msg.content_preview)}</span>
            </div>
        `).join('');

        if (document.getElementById('auto-scroll')?.checked) {
            stream.scrollTop = stream.scrollHeight;
        }
    }

    function clearMessages() {
        document.getElementById('message-stream').innerHTML =
            '<div class="no-messages">Messages cleared</div>';
    }

    function updateConnectionStatus(connected) {
        const indicator = document.getElementById('connection-status');
        const text = document.getElementById('status-text');

        indicator.classList.toggle('connected', connected);
        indicator.classList.toggle('disconnected', !connected);
        text.textContent = connected ? 'Connected' : 'Disconnected';
    }

    // Global functions for buttons
    window.suspendAgent = async function(name) {
        const result = await fetch(`/api/agents/${name}/suspend`, { method: 'POST' });
        const data = await result.json();
        alert(data.message);
        loadAgents();
    };

    window.viewAgent = function(name) {
        alert(`Agent details for: ${name}\n\nFull details view coming soon.`);
    };

    // Utility functions
    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text || '';
        return div.innerHTML;
    }

    function formatBytes(bytes) {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
    }

    function formatNumber(num) {
        return num.toLocaleString();
    }

    function formatUptime(seconds) {
        const days = Math.floor(seconds / 86400);
        const hours = Math.floor((seconds % 86400) / 3600);
        const mins = Math.floor((seconds % 3600) / 60);

        if (days > 0) return `${days}d ${hours}h`;
        if (hours > 0) return `${hours}h ${mins}m`;
        return `${mins}m`;
    }
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_config_default() {
        let config = DashboardConfig::default();
        assert_eq!(config.listen_addr.port(), 9091);
        assert!(config.enable_cors);
    }

    #[test]
    fn test_dashboard_state_default() {
        let state = DashboardState::default();
        assert!(state.agents.is_empty());
        assert!(state.services.is_empty());
        assert!(state.nodes.is_empty());
    }
}
