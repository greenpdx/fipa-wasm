// actor/registry.rs - Actor Name Registry

use actix::prelude::*;
use dashmap::DashMap;
use std::sync::Arc;
use tracing::{debug, info};

use crate::actor::messages::*;
use crate::actor::AgentActor;
use crate::proto;

/// Actor registry for name-based lookups
pub struct ActorRegistry {
    /// Local agent actors
    local_agents: Arc<DashMap<String, Addr<AgentActor>>>,

    /// Remote agent locations (agent_name -> node_id)
    remote_agents: Arc<DashMap<String, String>>,

    /// Service registry (service_name -> Vec<agent_name>)
    services: Arc<DashMap<String, Vec<String>>>,

    /// This node's ID
    node_id: String,
}

impl ActorRegistry {
    /// Create a new registry
    pub fn new(node_id: String) -> Self {
        Self {
            local_agents: Arc::new(DashMap::new()),
            remote_agents: Arc::new(DashMap::new()),
            services: Arc::new(DashMap::new()),
            node_id,
        }
    }

    /// Get a shared reference to local agents map
    pub fn local_agents(&self) -> Arc<DashMap<String, Addr<AgentActor>>> {
        self.local_agents.clone()
    }

    /// Check if agent is local
    pub fn is_local(&self, agent_name: &str) -> bool {
        self.local_agents.contains_key(agent_name)
    }

    /// Get node for an agent
    pub fn get_agent_node(&self, agent_name: &str) -> Option<String> {
        if self.local_agents.contains_key(agent_name) {
            Some(self.node_id.clone())
        } else {
            self.remote_agents.get(agent_name).map(|v| v.clone())
        }
    }

    /// Register a remote agent location
    pub fn register_remote(&self, agent_name: String, node_id: String) {
        self.remote_agents.insert(agent_name, node_id);
    }

    /// Deregister a remote agent
    pub fn deregister_remote(&self, agent_name: &str) {
        self.remote_agents.remove(agent_name);
    }

    /// Register a service provider
    pub fn register_service(&self, service_name: String, agent_name: String) {
        self.services
            .entry(service_name)
            .or_insert_with(Vec::new)
            .push(agent_name);
    }

    /// Find agents providing a service
    pub fn find_service_providers(&self, service_name: &str) -> Vec<String> {
        self.services
            .get(service_name)
            .map(|v| v.clone())
            .unwrap_or_default()
    }
}

impl Actor for ActorRegistry {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        info!("ActorRegistry started for node: {}", self.node_id);
    }
}

// =============================================================================
// Message Handlers
// =============================================================================

impl Handler<RegisterActor> for ActorRegistry {
    type Result = ();

    fn handle(&mut self, msg: RegisterActor, _ctx: &mut Self::Context) {
        debug!("Registering actor: {}", msg.agent_id.name);
        self.local_agents.insert(msg.agent_id.name.clone(), msg.addr);
    }
}

impl Handler<DeregisterActor> for ActorRegistry {
    type Result = ();

    fn handle(&mut self, msg: DeregisterActor, _ctx: &mut Self::Context) {
        debug!("Deregistering actor: {}", msg.agent_id.name);
        self.local_agents.remove(&msg.agent_id.name);
    }
}

impl Handler<LookupActor> for ActorRegistry {
    type Result = Option<Addr<AgentActor>>;

    fn handle(&mut self, msg: LookupActor, _ctx: &mut Self::Context) -> Self::Result {
        self.local_agents.get(&msg.agent_id.name).map(|v| v.clone())
    }
}

impl Handler<ResolveAgent> for ActorRegistry {
    type Result = Result<String, AgentError>;

    fn handle(&mut self, msg: ResolveAgent, _ctx: &mut Self::Context) -> Self::Result {
        self.get_agent_node(&msg.agent_id.name)
            .ok_or_else(|| AgentError::NotFound(msg.agent_id.name.clone()))
    }
}

impl Handler<FindAgents> for ActorRegistry {
    type Result = Result<Vec<proto::AgentId>, AgentError>;

    fn handle(&mut self, msg: FindAgents, _ctx: &mut Self::Context) -> Self::Result {
        let agent_names = self.find_service_providers(&msg.service_name);

        let agents: Vec<proto::AgentId> = agent_names
            .into_iter()
            .map(|name| proto::AgentId {
                name,
                addresses: vec![],
                resolvers: vec![],
            })
            .collect();

        Ok(agents)
    }
}

/// Message to update remote agent registry from consensus
#[derive(Message)]
#[rtype(result = "()")]
pub struct UpdateRemoteRegistry {
    pub agent_id: proto::AgentId,
    pub node_id: String,
    pub is_register: bool,
}

impl Handler<UpdateRemoteRegistry> for ActorRegistry {
    type Result = ();

    fn handle(&mut self, msg: UpdateRemoteRegistry, _ctx: &mut Self::Context) {
        if msg.is_register {
            self.register_remote(msg.agent_id.name, msg.node_id);
        } else {
            self.deregister_remote(&msg.agent_id.name);
        }
    }
}

/// Message to update service registry
#[derive(Message)]
#[rtype(result = "()")]
pub struct UpdateServiceRegistry {
    pub service_name: String,
    pub agent_name: String,
    pub is_register: bool,
}

impl Handler<UpdateServiceRegistry> for ActorRegistry {
    type Result = ();

    fn handle(&mut self, msg: UpdateServiceRegistry, _ctx: &mut Self::Context) {
        if msg.is_register {
            self.register_service(msg.service_name, msg.agent_name);
        } else {
            // Remove agent from service
            if let Some(mut providers) = self.services.get_mut(&msg.service_name) {
                providers.retain(|a| a != &msg.agent_name);
            }
        }
    }
}
