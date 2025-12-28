// platform/ams.rs - Agent Management System (AMS)
//
//! Agent Management System (AMS)
//!
//! The AMS is a mandatory FIPA platform agent that provides:
//! - Agent lifecycle management (create, destroy, suspend, resume)
//! - Agent naming service (ensures unique names)
//! - Platform access control and authentication
//! - Platform-wide agent directory
//!
//! # FIPA Compliance
//!
//! This implementation follows FIPA00023 (Agent Management Specification).
//! The AMS can be interacted with via ACL messages using:
//! - `request` performative for agent creation/destruction
//! - `query-ref` performative for agent queries

use actix::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{error, info};

use crate::actor::{
    AgentConfig, AgentError, RestartStrategy,
    ShutdownReason, SpawnAgent, StopAgent, Supervisor,
};
use crate::proto;

/// AMS configuration
#[derive(Debug, Clone)]
pub struct AMSConfig {
    /// Platform name (e.g., "fipa-platform")
    pub platform_name: String,

    /// Whether to enforce authentication for agent operations
    pub require_auth: bool,

    /// Maximum agents allowed on the platform (0 = unlimited)
    pub max_agents: usize,

    /// Reserved agent names that cannot be used
    pub reserved_names: HashSet<String>,

    /// Default capabilities for new agents
    pub default_capabilities: proto::AgentCapabilities,
}

impl Default for AMSConfig {
    fn default() -> Self {
        let mut reserved = HashSet::new();
        reserved.insert("ams".to_string());
        reserved.insert("df".to_string());
        reserved.insert("acc".to_string()); // Agent Communication Channel

        Self {
            platform_name: "fipa-platform".to_string(),
            require_auth: false,
            max_agents: 0,
            reserved_names: reserved,
            default_capabilities: proto::AgentCapabilities {
                max_memory_bytes: 64 * 1024 * 1024, // 64MB
                max_execution_time_ms: 30_000,      // 30 seconds
                allowed_protocols: vec![],          // All protocols
                network_access: proto::NetworkAccessLevel::NetworkAccessLocal as i32,
                storage_quota_bytes: 10 * 1024 * 1024, // 10MB
                migration_allowed: true,
                spawn_allowed: true,
                allowed_destinations: vec![],
            },
        }
    }
}

/// Agent registration record in AMS
#[derive(Debug, Clone)]
pub struct AgentRegistration {
    /// Agent identifier
    pub agent_id: proto::AgentId,

    /// Agent state
    pub state: AgentState,

    /// Registration timestamp
    pub registered_at: Instant,

    /// Owner/creator of this agent
    pub owner: Option<String>,

    /// Custom properties
    pub properties: HashMap<String, String>,
}

/// Agent state in AMS
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// Agent is being initialized
    Initiated,

    /// Agent is active and can receive messages
    Active,

    /// Agent is suspended (not processing messages)
    Suspended,

    /// Agent is waiting for other agents
    Waiting,

    /// Agent is in transit (migrating)
    Transit,

    /// Agent has terminated
    Terminated,
}

/// Agent Management System actor
pub struct AMS {
    /// Configuration
    config: AMSConfig,

    /// Registered agents (name -> registration)
    agents: HashMap<String, AgentRegistration>,

    /// Supervisor reference
    supervisor: Option<Addr<Supervisor>>,

    /// Agent ID of this AMS
    agent_id: proto::AgentId,

    /// Statistics
    stats: AMSStats,
}

/// AMS statistics
#[derive(Debug, Default, Clone)]
pub struct AMSStats {
    pub agents_created: u64,
    pub agents_destroyed: u64,
    pub agents_suspended: u64,
    pub agents_resumed: u64,
    pub queries_handled: u64,
    pub auth_failures: u64,
}

impl AMS {
    /// Create a new AMS
    pub fn new(config: AMSConfig) -> Self {
        let platform_name = config.platform_name.clone();
        Self {
            config,
            agents: HashMap::new(),
            supervisor: None,
            agent_id: proto::AgentId {
                name: "ams".to_string(),
                addresses: vec![format!("ams@{}", platform_name)],
                resolvers: vec![],
            },
            stats: AMSStats::default(),
        }
    }

    /// Set the supervisor reference
    pub fn with_supervisor(mut self, supervisor: Addr<Supervisor>) -> Self {
        self.supervisor = Some(supervisor);
        self
    }

    /// Get the AMS agent ID
    pub fn agent_id(&self) -> &proto::AgentId {
        &self.agent_id
    }

    /// Check if a name is available
    pub fn is_name_available(&self, name: &str) -> bool {
        !self.agents.contains_key(name) && !self.config.reserved_names.contains(name)
    }

    /// Generate a unique name
    pub fn generate_unique_name(&self, prefix: &str) -> String {
        let mut counter = 0;
        loop {
            let name = if counter == 0 {
                prefix.to_string()
            } else {
                format!("{}-{}", prefix, counter)
            };

            if self.is_name_available(&name) {
                return name;
            }
            counter += 1;
        }
    }

    /// Create an agent
    fn create_agent(&mut self, request: AMSCreateAgent, ctx: &mut Context<Self>) -> Result<proto::AgentId, AMSError> {
        // Validate name
        let name = if let Some(n) = &request.name {
            if !self.is_name_available(n) {
                return Err(AMSError::NameNotAvailable(n.clone()));
            }
            n.clone()
        } else {
            self.generate_unique_name("agent")
        };

        // Check agent limit
        if self.config.max_agents > 0 && self.agents.len() >= self.config.max_agents {
            return Err(AMSError::AgentLimitReached);
        }

        // Create agent ID
        let agent_id = proto::AgentId {
            name: name.clone(),
            addresses: vec![format!("{}@{}", name, self.config.platform_name)],
            resolvers: vec![],
        };

        // Create agent config
        let config = AgentConfig {
            id: agent_id.clone(),
            wasm_module: request.wasm_module,
            capabilities: request.capabilities.unwrap_or_else(|| self.config.default_capabilities.clone()),
            initial_state: None, // Initial state is set separately
            restart_strategy: RestartStrategy::default(),
        };

        // Spawn via supervisor
        if let Some(supervisor) = &self.supervisor {
            let supervisor = supervisor.clone();
            let agent_id_clone = agent_id.clone();
            let name_clone = name.clone();
            let owner = request.owner.clone();

            ctx.spawn(async move {
                match supervisor.send(SpawnAgent { config }).await {
                    Ok(Ok(_)) => {
                        info!("AMS: Agent '{}' created successfully", name_clone);
                    }
                    Ok(Err(e)) => {
                        error!("AMS: Failed to spawn agent '{}': {}", name_clone, e);
                    }
                    Err(e) => {
                        error!("AMS: Supervisor communication error: {}", e);
                    }
                }
            }.into_actor(self).map(move |_, act, _| {
                // Register agent in AMS
                act.agents.insert(name.clone(), AgentRegistration {
                    agent_id: agent_id_clone,
                    state: AgentState::Active,
                    registered_at: Instant::now(),
                    owner,
                    properties: HashMap::new(),
                });
                act.stats.agents_created += 1;
            }));
        } else {
            return Err(AMSError::SupervisorNotAvailable);
        }

        Ok(agent_id)
    }

    /// Destroy an agent
    fn destroy_agent(&mut self, request: AMSDestroyAgent) -> Result<(), AMSError> {
        let name = &request.agent_name;

        // Check if agent exists
        if !self.agents.contains_key(name) {
            return Err(AMSError::AgentNotFound(name.clone()));
        }

        // Check authorization
        if self.config.require_auth {
            if let Some(reg) = self.agents.get(name) {
                if let Some(owner) = &reg.owner {
                    if request.requester.as_ref() != Some(owner) {
                        return Err(AMSError::Unauthorized);
                    }
                }
            }
        }

        // Send stop command to supervisor
        if let Some(supervisor) = &self.supervisor {
            let agent_id = proto::AgentId {
                name: name.clone(),
                addresses: vec![],
                resolvers: vec![],
            };

            supervisor.do_send(StopAgent {
                agent_id,
                reason: ShutdownReason::Requested,
            });
        }

        // Update registration
        if let Some(reg) = self.agents.get_mut(name) {
            reg.state = AgentState::Terminated;
        }

        self.stats.agents_destroyed += 1;
        info!("AMS: Agent '{}' destroyed", name);

        Ok(())
    }

    /// Suspend an agent
    fn suspend_agent(&mut self, request: AMSSuspendAgent) -> Result<(), AMSError> {
        let name = &request.agent_name;

        if let Some(reg) = self.agents.get_mut(name) {
            if reg.state != AgentState::Active {
                return Err(AMSError::InvalidState(format!(
                    "Agent is {:?}, cannot suspend", reg.state
                )));
            }
            reg.state = AgentState::Suspended;
            self.stats.agents_suspended += 1;
            info!("AMS: Agent '{}' suspended", name);
            Ok(())
        } else {
            Err(AMSError::AgentNotFound(name.clone()))
        }
    }

    /// Resume an agent
    fn resume_agent(&mut self, request: AMSResumeAgent) -> Result<(), AMSError> {
        let name = &request.agent_name;

        if let Some(reg) = self.agents.get_mut(name) {
            if reg.state != AgentState::Suspended {
                return Err(AMSError::InvalidState(format!(
                    "Agent is {:?}, cannot resume", reg.state
                )));
            }
            reg.state = AgentState::Active;
            self.stats.agents_resumed += 1;
            info!("AMS: Agent '{}' resumed", name);
            Ok(())
        } else {
            Err(AMSError::AgentNotFound(name.clone()))
        }
    }

    /// Query agents
    fn query_agents(&mut self, request: AMSQueryAgents) -> Vec<AMSAgentDescription> {
        self.stats.queries_handled += 1;

        self.agents
            .iter()
            .filter(|(name, reg)| {
                // Filter by name pattern
                if let Some(pattern) = &request.name_pattern {
                    if !name.contains(pattern) {
                        return false;
                    }
                }

                // Filter by state
                if let Some(state) = &request.state {
                    if reg.state != *state {
                        return false;
                    }
                }

                // Filter by owner
                if let Some(owner) = &request.owner {
                    if reg.owner.as_ref() != Some(owner) {
                        return false;
                    }
                }

                true
            })
            .map(|(_, reg)| AMSAgentDescription {
                name: reg.agent_id.name.clone(),
                addresses: reg.agent_id.addresses.clone(),
                state: reg.state.clone(),
                owner: reg.owner.clone(),
            })
            .collect()
    }

    /// Get platform description
    pub fn get_platform_description(&self) -> PlatformDescription {
        PlatformDescription {
            name: self.config.platform_name.clone(),
            ams_address: format!("ams@{}", self.config.platform_name),
            df_address: format!("df@{}", self.config.platform_name),
            agent_count: self.agents.len(),
            max_agents: self.config.max_agents,
        }
    }
}

impl Actor for AMS {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        info!("AMS started for platform: {}", self.config.platform_name);

        // Register AMS itself
        self.agents.insert("ams".to_string(), AgentRegistration {
            agent_id: self.agent_id.clone(),
            state: AgentState::Active,
            registered_at: Instant::now(),
            owner: None,
            properties: HashMap::new(),
        });
    }
}

// =============================================================================
// Messages
// =============================================================================

/// Request to create an agent
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<proto::AgentId, AMSError>")]
pub struct AMSCreateAgent {
    /// Requested agent name (optional, will be generated if not provided)
    pub name: Option<String>,

    /// WASM module bytes
    pub wasm_module: Vec<u8>,

    /// Agent capabilities (optional, uses platform defaults)
    pub capabilities: Option<proto::AgentCapabilities>,

    /// Owner/creator of this agent
    pub owner: Option<String>,
}

/// Request to destroy an agent
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), AMSError>")]
pub struct AMSDestroyAgent {
    /// Agent name to destroy
    pub agent_name: String,

    /// Requester (for authorization)
    pub requester: Option<String>,
}

/// Request to suspend an agent
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), AMSError>")]
pub struct AMSSuspendAgent {
    /// Agent name to suspend
    pub agent_name: String,

    /// Requester (for authorization)
    pub requester: Option<String>,
}

/// Request to resume an agent
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), AMSError>")]
pub struct AMSResumeAgent {
    /// Agent name to resume
    pub agent_name: String,

    /// Requester (for authorization)
    pub requester: Option<String>,
}

/// Query agents on the platform
#[derive(Debug, Clone, Default, Message)]
#[rtype(result = "Vec<AMSAgentDescription>")]
pub struct AMSQueryAgents {
    /// Name pattern filter
    pub name_pattern: Option<String>,

    /// State filter
    pub state: Option<AgentState>,

    /// Owner filter
    pub owner: Option<String>,
}

/// Get platform description
#[derive(Debug, Clone, Message)]
#[rtype(result = "PlatformDescription")]
pub struct GetPlatformDescription;

// =============================================================================
// Response Types
// =============================================================================

/// Agent description returned by AMS queries
#[derive(Debug, Clone)]
pub struct AMSAgentDescription {
    /// Agent name
    pub name: String,

    /// Agent addresses
    pub addresses: Vec<String>,

    /// Current state
    pub state: AgentState,

    /// Owner
    pub owner: Option<String>,
}

/// Platform description
#[derive(Debug, Clone, MessageResponse)]
pub struct PlatformDescription {
    /// Platform name
    pub name: String,

    /// AMS address
    pub ams_address: String,

    /// DF address
    pub df_address: String,

    /// Current agent count
    pub agent_count: usize,

    /// Maximum agents (0 = unlimited)
    pub max_agents: usize,
}

// =============================================================================
// Errors
// =============================================================================

/// AMS errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum AMSError {
    #[error("Agent name not available: {0}")]
    NameNotAvailable(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Agent limit reached")]
    AgentLimitReached,

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Supervisor not available")]
    SupervisorNotAvailable,

    #[error("Agent error: {0}")]
    AgentError(String),
}

impl From<AgentError> for AMSError {
    fn from(e: AgentError) -> Self {
        AMSError::AgentError(e.to_string())
    }
}

// =============================================================================
// Message Handlers
// =============================================================================

impl Handler<AMSCreateAgent> for AMS {
    type Result = Result<proto::AgentId, AMSError>;

    fn handle(&mut self, msg: AMSCreateAgent, ctx: &mut Self::Context) -> Self::Result {
        self.create_agent(msg, ctx)
    }
}

impl Handler<AMSDestroyAgent> for AMS {
    type Result = Result<(), AMSError>;

    fn handle(&mut self, msg: AMSDestroyAgent, _ctx: &mut Self::Context) -> Self::Result {
        self.destroy_agent(msg)
    }
}

impl Handler<AMSSuspendAgent> for AMS {
    type Result = Result<(), AMSError>;

    fn handle(&mut self, msg: AMSSuspendAgent, _ctx: &mut Self::Context) -> Self::Result {
        self.suspend_agent(msg)
    }
}

impl Handler<AMSResumeAgent> for AMS {
    type Result = Result<(), AMSError>;

    fn handle(&mut self, msg: AMSResumeAgent, _ctx: &mut Self::Context) -> Self::Result {
        self.resume_agent(msg)
    }
}

impl Handler<AMSQueryAgents> for AMS {
    type Result = Vec<AMSAgentDescription>;

    fn handle(&mut self, msg: AMSQueryAgents, _ctx: &mut Self::Context) -> Self::Result {
        self.query_agents(msg)
    }
}

impl Handler<GetPlatformDescription> for AMS {
    type Result = PlatformDescription;

    fn handle(&mut self, _msg: GetPlatformDescription, _ctx: &mut Self::Context) -> Self::Result {
        self.get_platform_description()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ams_config_default() {
        let config = AMSConfig::default();
        assert_eq!(config.platform_name, "fipa-platform");
        assert!(config.reserved_names.contains("ams"));
        assert!(config.reserved_names.contains("df"));
    }

    #[test]
    fn test_name_availability() {
        let ams = AMS::new(AMSConfig::default());
        assert!(!ams.is_name_available("ams")); // Reserved
        assert!(ams.is_name_available("my-agent"));
    }

    #[test]
    fn test_generate_unique_name() {
        let ams = AMS::new(AMSConfig::default());
        let name = ams.generate_unique_name("test");
        assert_eq!(name, "test");
    }
}
