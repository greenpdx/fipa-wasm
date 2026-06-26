// actor/supervisor.rs - Agent Supervision Tree

use actix::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::actor::messages::*;
use crate::actor::AgentActor;
use crate::proto;
use crate::wasm::WasmRuntime;

/// Supervisor actor managing agent lifecycle
pub struct Supervisor {
    /// Supervised agents
    agents: HashMap<String, SupervisedAgent>,

    /// Network actor for messaging
    network: Option<Addr<super::NetworkActor>>,

    /// Actor registry
    registry: Option<Addr<super::ActorRegistry>>,

    /// Node ID for this supervisor
    node_id: String,
}

/// State for a supervised agent
struct SupervisedAgent {
    /// Agent address
    addr: Addr<AgentActor>,

    /// Agent configuration
    config: AgentConfig,

    /// Restart strategy
    strategy: RestartStrategy,

    /// Failure history
    failures: Vec<Instant>,

    /// Current backoff delay
    current_backoff: Duration,

    /// Last known state
    state: AgentRuntimeState,
}

impl Supervisor {
    /// Create a new supervisor
    pub fn new(node_id: String) -> Self {
        Self {
            agents: HashMap::new(),
            network: None,
            registry: None,
            node_id,
        }
    }

    /// Set the network actor
    pub fn with_network(mut self, network: Addr<super::NetworkActor>) -> Self {
        self.network = Some(network);
        self
    }

    /// Set the registry
    pub fn with_registry(mut self, registry: Addr<super::ActorRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Spawn an agent under supervision
    fn spawn_agent(&mut self, config: AgentConfig, ctx: &mut Context<Self>) -> Result<Addr<AgentActor>, AgentError> {
        let agent_name = config.id.name.clone();

        // Check if agent already exists
        if self.agents.contains_key(&agent_name) {
            return Err(AgentError::InvalidState(format!(
                "Agent {} already exists",
                agent_name
            )));
        }

        // Create WASM runtime
        let mut runtime = WasmRuntime::new(&config.wasm_module, &config.capabilities)
            .map_err(|e| AgentError::RuntimeError(e.to_string()))?;

        // Restore state if provided
        if let Some(state) = &config.initial_state {
            runtime.restore_state(state)
                .map_err(|e| AgentError::RuntimeError(e.to_string()))?;
        }

        // Create and start actor
        let mut actor = AgentActor::new(config.clone(), runtime)
            .with_supervisor(ctx.address());

        if let Some(network) = &self.network {
            actor = actor.with_network(network.clone());
        }

        if let Some(registry) = &self.registry {
            actor = actor.with_registry(registry.clone());
        }

        let addr = actor.start();

        // Track supervised agent
        self.agents.insert(agent_name.clone(), SupervisedAgent {
            addr: addr.clone(),
            config,
            strategy: RestartStrategy::default(),
            failures: Vec::new(),
            current_backoff: Duration::from_secs(1),
            state: AgentRuntimeState::Starting,
        });

        info!("Spawned agent: {}", agent_name);
        Ok(addr)
    }

    /// Handle agent failure
    fn handle_failure(&mut self, agent_id: &proto::AgentId, _error: String, ctx: &mut Context<Self>) {
        let agent_name = &agent_id.name;

        if let Some(supervised) = self.agents.get_mut(agent_name) {
            supervised.failures.push(Instant::now());
            supervised.state = AgentRuntimeState::Failed;

            // Determine if we should restart
            let should_restart = match &supervised.strategy {
                RestartStrategy::Immediate => true,
                RestartStrategy::Backoff { .. } => true,
                RestartStrategy::MaxFailures { count, window } => {
                    // Count failures within window
                    let cutoff = Instant::now() - *window;
                    supervised.failures.retain(|t| *t > cutoff);
                    supervised.failures.len() < *count as usize
                }
                RestartStrategy::Never => false,
            };

            if should_restart {
                let delay = match &supervised.strategy {
                    RestartStrategy::Immediate => Duration::ZERO,
                    RestartStrategy::Backoff { initial: _, max, multiplier } => {
                        let delay = supervised.current_backoff;
                        supervised.current_backoff = Duration::from_secs_f64(
                            (supervised.current_backoff.as_secs_f64() * multiplier).min(max.as_secs_f64())
                        );
                        delay
                    }
                    _ => Duration::from_secs(1),
                };

                info!("Scheduling restart for {} in {:?}", agent_name, delay);

                let agent_id = agent_id.clone();
                ctx.run_later(delay, move |actor, ctx| {
                    if let Some(supervised) = actor.agents.get(&agent_id.name) {
                        // Attempt restart
                        let config = supervised.config.clone();
                        match actor.spawn_agent(config, ctx) {
                            Ok(_) => {
                                info!("Successfully restarted agent: {}", agent_id.name);
                            }
                            Err(e) => {
                                error!("Failed to restart agent {}: {}", agent_id.name, e);
                            }
                        }
                    }
                });
            } else {
                warn!("Agent {} exceeded restart limit, not restarting", agent_name);
            }
        }
    }

    /// Stop an agent
    fn stop_agent(&mut self, agent_id: &proto::AgentId, reason: ShutdownReason) -> Result<(), AgentError> {
        let agent_name = &agent_id.name;

        if let Some(supervised) = self.agents.remove(agent_name) {
            supervised.addr.do_send(Shutdown { reason });
            info!("Stopped agent: {}", agent_name);
            Ok(())
        } else {
            Err(AgentError::NotFound(agent_name.clone()))
        }
    }

    /// Get list of all agents
    fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents.iter().map(|(_name, supervised)| {
            AgentInfo {
                agent_id: supervised.config.id.clone(),
                state: supervised.state.clone(),
                restart_count: supervised.failures.len() as u32,
                last_error: None, // Could track last error
            }
        }).collect()
    }
}

impl Actor for Supervisor {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        info!("Supervisor started for node: {}", self.node_id);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        info!("Supervisor stopping, shutting down all agents");

        // Stop all agents
        for (name, supervised) in &self.agents {
            supervised.addr.do_send(Shutdown {
                reason: ShutdownReason::NodeShutdown,
            });
            info!("Sent shutdown to agent: {}", name);
        }

        Running::Stop
    }
}

// =============================================================================
// Message Handlers
// =============================================================================

impl Handler<SpawnAgent> for Supervisor {
    type Result = Result<Addr<AgentActor>, AgentError>;

    fn handle(&mut self, msg: SpawnAgent, ctx: &mut Self::Context) -> Self::Result {
        self.spawn_agent(msg.config, ctx)
    }
}

impl Handler<StopAgent> for Supervisor {
    type Result = Result<(), AgentError>;

    fn handle(&mut self, msg: StopAgent, _ctx: &mut Self::Context) -> Self::Result {
        self.stop_agent(&msg.agent_id, msg.reason)
    }
}

impl Handler<ListAgents> for Supervisor {
    type Result = Vec<AgentInfo>;

    fn handle(&mut self, _msg: ListAgents, _ctx: &mut Self::Context) -> Self::Result {
        self.list_agents()
    }
}

impl Handler<SupervisionEvent> for Supervisor {
    type Result = ();

    fn handle(&mut self, msg: SupervisionEvent, ctx: &mut Self::Context) {
        let agent_name = &msg.agent_id.name;
        debug!("Supervision event for {}: {:?}", agent_name, msg.event);

        match msg.event {
            SupervisionEventType::Started => {
                if let Some(supervised) = self.agents.get_mut(agent_name) {
                    supervised.state = AgentRuntimeState::Running;
                    // Reset backoff on successful start
                    if let RestartStrategy::Backoff { initial, .. } = &supervised.strategy {
                        supervised.current_backoff = *initial;
                    }
                }
            }
            SupervisionEventType::Stopped => {
                if let Some(supervised) = self.agents.get_mut(agent_name) {
                    supervised.state = AgentRuntimeState::Stopped;
                }
            }
            SupervisionEventType::Failed { error, will_restart } => {
                if will_restart {
                    self.handle_failure(&msg.agent_id, error, ctx);
                }
            }
            SupervisionEventType::Migrated { from_node, to_node } => {
                info!("Agent {} migrated from {} to {}", agent_name, from_node, to_node);
                // Remove from local supervision
                self.agents.remove(agent_name);
            }
            SupervisionEventType::Recovered => {
                if let Some(supervised) = self.agents.get_mut(agent_name) {
                    supervised.state = AgentRuntimeState::Running;
                }
            }
        }
    }
}

impl Handler<DeliverMessage> for Supervisor {
    type Result = Result<(), AgentError>;

    fn handle(&mut self, msg: DeliverMessage, _ctx: &mut Self::Context) -> Self::Result {
        // Route message to appropriate agent
        for receiver in &msg.message.receivers {
            if let Some(supervised) = self.agents.get(&receiver.name) {
                supervised.addr.do_send(msg.clone());
            } else {
                warn!("Agent not found for message delivery: {}", receiver.name);
            }
        }
        Ok(())
    }
}
