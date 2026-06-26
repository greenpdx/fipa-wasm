// network/grpc.rs - gRPC Service Implementations

use actix::Addr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::actor::{ActorRegistry, DeliverMessage, FindAgents, Supervisor};
use crate::proto;
use crate::proto::fipa_agent_service_server::FipaAgentService;
use crate::proto::consensus_service_server::ConsensusService;

// =============================================================================
// FIPA Agent Service Implementation
// =============================================================================

/// State shared across gRPC service instances
pub struct ServiceState {
    /// Node ID
    pub node_id: String,

    /// Supervisor actor
    pub supervisor: Addr<Supervisor>,

    /// Actor registry
    pub registry: Addr<ActorRegistry>,

    /// Node capabilities
    pub capabilities: proto::NodeCapabilities,

    /// Current node metrics
    pub metrics: Arc<RwLock<proto::NodeMetrics>>,
}

impl ServiceState {
    pub fn new(
        node_id: String,
        supervisor: Addr<Supervisor>,
        registry: Addr<ActorRegistry>,
    ) -> Self {
        Self {
            node_id,
            supervisor,
            registry,
            capabilities: proto::NodeCapabilities {
                max_agents: 100,
                total_memory: 1024 * 1024 * 1024, // 1GB
                supported_protocols: vec![
                    proto::ProtocolType::ProtocolRequest as i32,
                    proto::ProtocolType::ProtocolQuery as i32,
                    proto::ProtocolType::ProtocolContractNet as i32,
                    proto::ProtocolType::ProtocolSubscribe as i32,
                ],
                security_level: proto::SecurityLevel::Trusted as i32,
                wasm_runtime_version: "wasmtime-40".into(),
            },
            metrics: Arc::new(RwLock::new(proto::NodeMetrics::default())),
        }
    }

    /// Update node metrics
    pub async fn update_metrics(&self, metrics: proto::NodeMetrics) {
        let mut m = self.metrics.write().await;
        *m = metrics;
    }
}

/// gRPC implementation of FipaAgentService
pub struct FipaAgentServiceImpl {
    state: Arc<ServiceState>,
}

impl FipaAgentServiceImpl {
    pub fn new(state: Arc<ServiceState>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl FipaAgentService for FipaAgentServiceImpl {
    /// Send an ACL message to an agent
    async fn send_message(
        &self,
        request: Request<proto::AclMessage>,
    ) -> Result<Response<proto::SendMessageResponse>, Status> {
        let msg = request.into_inner();
        let message_id = msg.message_id.clone();

        debug!("Received message: {}", message_id);

        // Validate message has receivers
        if msg.receivers.is_empty() {
            return Ok(Response::new(proto::SendMessageResponse {
                success: false,
                message_id: message_id.clone(),
                error: Some("No receivers specified".into()),
            }));
        }

        // Try to deliver to each receiver
        let mut delivered = false;
        for receiver in &msg.receivers {
            // Look up the agent in registry
            let lookup_result = self.state.registry
                .send(crate::actor::LookupActor {
                    agent_id: receiver.clone(),
                })
                .await;

            match lookup_result {
                Ok(Some(agent_addr)) => {
                    // Deliver message to local agent
                    let deliver_result = agent_addr
                        .send(DeliverMessage { message: msg.clone() })
                        .await;

                    match deliver_result {
                        Ok(Ok(())) => {
                            delivered = true;
                            debug!("Delivered message to agent: {}", receiver.name);
                        }
                        Ok(Err(e)) => {
                            warn!("Agent rejected message: {}", e);
                        }
                        Err(e) => {
                            error!("Mailbox error: {}", e);
                        }
                    }
                }
                Ok(None) => {
                    debug!("Agent not found locally: {}", receiver.name);
                    // Could forward to other nodes here
                }
                Err(e) => {
                    error!("Registry lookup failed: {}", e);
                }
            }
        }

        Ok(Response::new(proto::SendMessageResponse {
            success: delivered,
            message_id,
            error: if delivered { None } else { Some("No local agents found".into()) },
        }))
    }

    type SubscribeMessagesStream = ReceiverStream<Result<proto::AclMessage, Status>>;

    /// Stream messages for an agent (server streaming)
    async fn subscribe_messages(
        &self,
        request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeMessagesStream>, Status> {
        let req = request.into_inner();
        let _agent_id = req.agent_id;
        let _filter_conversation = req.filter_conversation;
        let _filter_protocol = req.filter_protocol;

        info!("New message subscription requested");

        // Create a channel for streaming messages
        // In a full implementation, this would register with the actor system
        // to receive messages for the specified agent
        let (_tx, rx) = mpsc::channel(32);

        // TODO: Register subscription with supervisor/registry to receive messages
        // For now, return an empty stream that the client can listen on

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    /// Find an agent's location
    async fn find_agent(
        &self,
        request: Request<proto::FindAgentRequest>,
    ) -> Result<Response<proto::FindAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = req.agent_id
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;

        debug!("Finding agent: {}", agent_id.name);

        // Check if agent is local
        let lookup_result = self.state.registry
            .send(crate::actor::LookupActor {
                agent_id: agent_id.clone(),
            })
            .await;

        match lookup_result {
            Ok(Some(_)) => {
                // Agent is local
                let metrics = self.state.metrics.read().await.clone();
                Ok(Response::new(proto::FindAgentResponse {
                    found: true,
                    node_id: Some(self.state.node_id.clone()),
                    node_info: Some(proto::NodeAnnouncement {
                        node_id: self.state.node_id.clone(),
                        addresses: vec![], // Would fill from transport
                        capabilities: Some(self.state.capabilities.clone()),
                        metrics: Some(metrics),
                        timestamp: chrono::Utc::now().timestamp_millis(),
                    }),
                }))
            }
            Ok(None) => {
                // Agent not found locally - could query other nodes
                Ok(Response::new(proto::FindAgentResponse {
                    found: false,
                    node_id: None,
                    node_info: None,
                }))
            }
            Err(e) => {
                Err(Status::internal(format!("Registry error: {}", e)))
            }
        }
    }

    /// Find agents providing a service
    async fn find_service(
        &self,
        request: Request<proto::FindServiceRequest>,
    ) -> Result<Response<proto::FindServiceResponse>, Status> {
        let req = request.into_inner();

        debug!("Finding service: {}", req.service_name);

        // Query registry for agents with this service
        let find_result = self.state.registry
            .send(FindAgents {
                service_name: req.service_name.clone(),
                protocol: req.required_protocol.and_then(|p| proto::ProtocolType::try_from(p).ok()),
            })
            .await;

        match find_result {
            Ok(Ok(agents)) => {
                let metrics = self.state.metrics.read().await.clone();
                let providers: Vec<proto::ServiceProvider> = agents
                    .into_iter()
                    .take(req.max_results as usize)
                    .map(|agent_id| {
                        proto::ServiceProvider {
                            agent_id: Some(agent_id),
                            node_id: self.state.node_id.clone(),
                            service: Some(proto::ServiceDescription {
                                name: req.service_name.clone(),
                                description: String::new(),
                                protocols: vec![],
                                ontology: req.ontology.clone().unwrap_or_default(),
                                properties: std::collections::HashMap::new(),
                            }),
                            node_metrics: Some(metrics.clone()),
                        }
                    })
                    .collect();

                Ok(Response::new(proto::FindServiceResponse { providers }))
            }
            Ok(Err(e)) => {
                Err(Status::internal(format!("Registry error: {}", e)))
            }
            Err(e) => {
                Err(Status::internal(format!("Mailbox error: {}", e)))
            }
        }
    }

    /// Migrate an agent to this node
    async fn migrate_agent(
        &self,
        request: Request<proto::AgentMigration>,
    ) -> Result<Response<proto::MigrationResponse>, Status> {
        let migration = request.into_inner();

        let agent_id = migration.agent_id
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;

        info!("Receiving migrated agent: {}", agent_id.name);

        // Validate migration package
        if migration.wasm_hash.is_empty() {
            return Err(Status::invalid_argument("wasm_hash required"));
        }

        // Get WASM module (either from package or cache)
        let wasm_module = if let Some(module) = migration.wasm_module {
            module
        } else {
            // Try to fetch from cache by hash
            return Err(Status::not_found("WASM module not provided and not in cache"));
        };

        // Spawn the agent
        let config = crate::actor::AgentConfig {
            id: agent_id.clone(),
            wasm_module,
            capabilities: migration.capabilities.unwrap_or_default(),
            initial_state: migration.state,
            restart_strategy: crate::actor::RestartStrategy::default(),
        };

        let spawn_result = self.state.supervisor
            .send(crate::actor::SpawnAgent { config })
            .await;

        match spawn_result {
            Ok(Ok(_addr)) => {
                info!("Agent migrated successfully: {}", agent_id.name);
                Ok(Response::new(proto::MigrationResponse {
                    success: true,
                    error: None,
                    new_location: Some(self.state.node_id.clone()),
                }))
            }
            Ok(Err(e)) => {
                error!("Failed to spawn migrated agent: {}", e);
                Ok(Response::new(proto::MigrationResponse {
                    success: false,
                    error: Some(e.to_string()),
                    new_location: None,
                }))
            }
            Err(e) => {
                Err(Status::internal(format!("Supervisor error: {}", e)))
            }
        }
    }

    /// Clone an agent to this node
    async fn clone_agent(
        &self,
        request: Request<proto::AgentMigration>,
    ) -> Result<Response<proto::MigrationResponse>, Status> {
        let mut migration = request.into_inner();

        // For cloning, generate a new unique agent ID
        if let Some(ref mut agent_id) = migration.agent_id {
            agent_id.name = format!("{}-clone-{}", agent_id.name, uuid::Uuid::new_v4());
        }

        // Reuse migrate logic
        self.migrate_agent(Request::new(migration)).await
    }

    /// Request WASM module by hash
    async fn get_wasm_module(
        &self,
        request: Request<proto::WasmModuleRequest>,
    ) -> Result<Response<proto::WasmModuleResponse>, Status> {
        let req = request.into_inner();

        debug!("Fetching WASM module by hash: {:?}", req.hash);

        // TODO: Implement WASM module cache lookup
        // For now, return not found
        Ok(Response::new(proto::WasmModuleResponse {
            found: false,
            module: None,
        }))
    }

    /// Health check
    async fn health_check(
        &self,
        request: Request<proto::HealthCheckRequest>,
    ) -> Result<Response<proto::HealthCheckResponse>, Status> {
        let req = request.into_inner();

        let metrics = if req.include_metrics {
            Some(self.state.metrics.read().await.clone())
        } else {
            None
        };

        Ok(Response::new(proto::HealthCheckResponse {
            healthy: true,
            status: "OK".into(),
            metrics,
        }))
    }

    /// Get node info
    async fn get_node_info(
        &self,
        _request: Request<proto::NodeInfoRequest>,
    ) -> Result<Response<proto::NodeAnnouncement>, Status> {
        let metrics = self.state.metrics.read().await.clone();

        Ok(Response::new(proto::NodeAnnouncement {
            node_id: self.state.node_id.clone(),
            addresses: vec![], // Would fill from transport
            capabilities: Some(self.state.capabilities.clone()),
            metrics: Some(metrics),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }))
    }
}

// =============================================================================
// Consensus Service Implementation
// =============================================================================

/// Raft consensus state (placeholder for openraft integration)
pub struct ConsensusState {
    /// Current term
    pub term: u64,

    /// Current leader ID (None if unknown)
    pub leader_id: Option<String>,

    /// Node ID
    pub node_id: String,

    /// Voted for in current term
    pub voted_for: Option<String>,

    /// Commit index
    pub commit_index: u64,

    /// Last applied index
    pub last_applied: u64,
}

impl ConsensusState {
    pub fn new(node_id: String) -> Self {
        Self {
            term: 0,
            leader_id: None,
            node_id,
            voted_for: None,
            commit_index: 0,
            last_applied: 0,
        }
    }
}

impl Default for ConsensusState {
    fn default() -> Self {
        Self::new("default".into())
    }
}

/// gRPC implementation of ConsensusService
pub struct ConsensusServiceImpl {
    state: Arc<RwLock<ConsensusState>>,
}

impl ConsensusServiceImpl {
    pub fn new(node_id: String) -> Self {
        Self {
            state: Arc::new(RwLock::new(ConsensusState::new(node_id))),
        }
    }

    pub fn with_state(state: Arc<RwLock<ConsensusState>>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ConsensusService for ConsensusServiceImpl {
    /// Raft vote request
    async fn vote(
        &self,
        request: Request<proto::VoteRequest>,
    ) -> Result<Response<proto::VoteResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.write().await;

        let _last_log_term = req.last_log_id.as_ref().map(|id| id.term).unwrap_or(0);

        debug!(
            "Vote request from {} for term {} (current term: {})",
            req.candidate_id, req.term, state.term
        );

        // Basic Raft voting logic
        let vote_granted = if req.term < state.term {
            // Candidate's term is old
            false
        } else if req.term > state.term {
            // Update to new term
            state.term = req.term;
            state.voted_for = Some(req.candidate_id.to_string());
            true
        } else {
            // Same term - check if already voted
            match &state.voted_for {
                None => {
                    state.voted_for = Some(req.candidate_id.to_string());
                    true
                }
                Some(voted) => voted == &req.candidate_id.to_string(),
            }
        };

        Ok(Response::new(proto::VoteResponse {
            vote_granted,
            term: state.term,
            data: vec![], // Serialized openraft response goes here
        }))
    }

    /// Raft append entries (heartbeat/replication)
    async fn append(
        &self,
        request: Request<proto::AppendRequest>,
    ) -> Result<Response<proto::AppendResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.write().await;

        debug!(
            "AppendEntries from leader {} for term {}",
            req.leader_id,
            req.term
        );

        // Update leader
        state.leader_id = Some(req.leader_id.to_string());

        // Basic append entries logic
        let success = if req.term < state.term {
            // Leader's term is old
            false
        } else {
            // Accept entries
            if req.term > state.term {
                state.term = req.term;
                state.voted_for = None;
            }

            // TODO: Actually process entries from req.entries (serialized openraft data)

            true
        };

        Ok(Response::new(proto::AppendResponse {
            success,
            term: state.term,
            data: vec![], // Serialized openraft response goes here
        }))
    }

    /// Raft install snapshot
    async fn snapshot(
        &self,
        request: Request<proto::SnapshotRequest>,
    ) -> Result<Response<proto::SnapshotResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.write().await;

        info!(
            "InstallSnapshot from {} with id {}",
            req.leader_id, req.snapshot_id
        );

        // Update term if needed
        if req.term > state.term {
            state.term = req.term;
            state.voted_for = None;
        }

        // TODO: Actually apply snapshot from req.data

        Ok(Response::new(proto::SnapshotResponse {
            success: true,
            term: state.term,
            data: vec![], // Serialized openraft response goes here
        }))
    }
}

// =============================================================================
// Server Builder
// =============================================================================

use tonic::transport::Server;

/// Configuration for the gRPC server
pub struct GrpcServerConfig {
    /// Address to bind to
    pub addr: std::net::SocketAddr,

    /// Enable TLS
    pub tls_enabled: bool,

    /// TLS certificate path
    pub cert_path: Option<String>,

    /// TLS key path
    pub key_path: Option<String>,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:50051".parse().unwrap(),
            tls_enabled: false,
            cert_path: None,
            key_path: None,
        }
    }
}

/// Build and run the gRPC server
pub async fn run_grpc_server(
    config: GrpcServerConfig,
    service_state: Arc<ServiceState>,
    consensus_state: Arc<RwLock<ConsensusState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let agent_service = FipaAgentServiceImpl::new(service_state);
    let consensus_service = ConsensusServiceImpl::with_state(consensus_state);

    info!("Starting gRPC server on {}", config.addr);

    Server::builder()
        .add_service(proto::fipa_agent_service_server::FipaAgentServiceServer::new(agent_service))
        .add_service(proto::consensus_service_server::ConsensusServiceServer::new(consensus_service))
        .serve(config.addr)
        .await?;

    Ok(())
}

// =============================================================================
// Standalone FIPA Agent Service (no actor system required)
// =============================================================================

/// Standalone configuration for FipaAgentService
pub struct StandaloneServiceConfig {
    pub node_id: String,
    pub node_name: String,
    pub grpc_addr: String,
}

impl Default for StandaloneServiceConfig {
    fn default() -> Self {
        Self {
            node_id: "1".into(),
            node_name: "node-1".into(),
            grpc_addr: "0.0.0.0:9000".into(),
        }
    }
}

/// Standalone gRPC implementation of FipaAgentService
/// Works without the actor system for basic testing and standalone mode
pub struct StandaloneFipaService {
    config: StandaloneServiceConfig,
    #[allow(dead_code)]
    start_time: std::time::Instant,
}

impl StandaloneFipaService {
    pub fn new(config: StandaloneServiceConfig) -> Self {
        Self {
            config,
            start_time: std::time::Instant::now(),
        }
    }
}

#[tonic::async_trait]
impl FipaAgentService for StandaloneFipaService {
    async fn send_message(
        &self,
        request: Request<proto::AclMessage>,
    ) -> Result<Response<proto::SendMessageResponse>, Status> {
        let msg = request.into_inner();
        debug!("Received message: {} (standalone mode - not delivered)", msg.message_id);

        Ok(Response::new(proto::SendMessageResponse {
            success: false,
            message_id: msg.message_id,
            error: Some("Standalone mode - no agents available".into()),
        }))
    }

    type SubscribeMessagesStream = ReceiverStream<Result<proto::AclMessage, Status>>;

    async fn subscribe_messages(
        &self,
        _request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeMessagesStream>, Status> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn find_agent(
        &self,
        request: Request<proto::FindAgentRequest>,
    ) -> Result<Response<proto::FindAgentResponse>, Status> {
        let req = request.into_inner();
        let agent_id = req.agent_id
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;

        debug!("Finding agent: {} (standalone mode)", agent_id.name);

        Ok(Response::new(proto::FindAgentResponse {
            found: false,
            node_id: None,
            node_info: None,
        }))
    }

    async fn find_service(
        &self,
        request: Request<proto::FindServiceRequest>,
    ) -> Result<Response<proto::FindServiceResponse>, Status> {
        let req = request.into_inner();
        debug!("Finding service: {} (standalone mode)", req.service_name);

        Ok(Response::new(proto::FindServiceResponse {
            providers: vec![],
        }))
    }

    async fn migrate_agent(
        &self,
        request: Request<proto::AgentMigration>,
    ) -> Result<Response<proto::MigrationResponse>, Status> {
        let migration = request.into_inner();
        let agent_id = migration.agent_id
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;

        info!("Migration request for agent: {} (standalone mode - rejected)", agent_id.name);

        Ok(Response::new(proto::MigrationResponse {
            success: false,
            error: Some("Standalone mode - migrations not supported".into()),
            new_location: None,
        }))
    }

    async fn clone_agent(
        &self,
        request: Request<proto::AgentMigration>,
    ) -> Result<Response<proto::MigrationResponse>, Status> {
        let migration = request.into_inner();
        let agent_id = migration.agent_id
            .ok_or_else(|| Status::invalid_argument("agent_id required"))?;

        info!("Clone request for agent: {} (standalone mode - rejected)", agent_id.name);

        Ok(Response::new(proto::MigrationResponse {
            success: false,
            error: Some("Standalone mode - cloning not supported".into()),
            new_location: None,
        }))
    }

    async fn get_wasm_module(
        &self,
        request: Request<proto::WasmModuleRequest>,
    ) -> Result<Response<proto::WasmModuleResponse>, Status> {
        let req = request.into_inner();
        debug!("WASM module request: {:?} (standalone mode)", req.hash);

        Ok(Response::new(proto::WasmModuleResponse {
            found: false,
            module: None,
        }))
    }

    async fn health_check(
        &self,
        _request: Request<proto::HealthCheckRequest>,
    ) -> Result<Response<proto::HealthCheckResponse>, Status> {
        Ok(Response::new(proto::HealthCheckResponse {
            healthy: true,
            status: "OK".into(),
            metrics: Some(proto::NodeMetrics {
                cpu_usage_percent: 0.0,
                memory_used_bytes: 0,
                memory_available_bytes: 1024 * 1024 * 1024, // 1GB
                active_agents: 0,
                active_conversations: 0,
                messages_sent: 0,
                messages_received: 0,
                average_latency_ms: 0,
            }),
        }))
    }

    async fn get_node_info(
        &self,
        _request: Request<proto::NodeInfoRequest>,
    ) -> Result<Response<proto::NodeAnnouncement>, Status> {
        Ok(Response::new(proto::NodeAnnouncement {
            node_id: self.config.node_id.clone(),
            addresses: vec![self.config.grpc_addr.clone()],
            capabilities: Some(proto::NodeCapabilities {
                max_agents: 100,
                total_memory: 1024 * 1024 * 1024,
                supported_protocols: vec![
                    proto::ProtocolType::ProtocolRequest as i32,
                    proto::ProtocolType::ProtocolQuery as i32,
                    proto::ProtocolType::ProtocolContractNet as i32,
                    proto::ProtocolType::ProtocolSubscribe as i32,
                ],
                security_level: proto::SecurityLevel::Trusted as i32,
                wasm_runtime_version: env!("CARGO_PKG_VERSION").into(),
            }),
            metrics: Some(proto::NodeMetrics {
                cpu_usage_percent: 0.0,
                memory_used_bytes: 0,
                memory_available_bytes: 1024 * 1024 * 1024, // 1GB
                active_agents: 0,
                active_conversations: 0,
                messages_sent: 0,
                messages_received: 0,
                average_latency_ms: 0,
            }),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_config_default() {
        let config = GrpcServerConfig::default();
        assert_eq!(config.addr.port(), 50051);
        assert!(!config.tls_enabled);
    }
}
