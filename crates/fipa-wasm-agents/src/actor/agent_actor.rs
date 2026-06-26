// actor/agent_actor.rs - WASM Agent Actor

use actix::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn, instrument};

use crate::actor::messages::*;
use crate::observability::{record_agent_spawned, record_agent_stopped, record_message_received};
use crate::proto;
use crate::wasm::WasmRuntime;
use crate::protocol::ProtocolStateMachine;

/// Actor wrapping a WASM agent instance
pub struct AgentActor {
    /// Agent identifier
    agent_id: proto::AgentId,

    /// WASM runtime instance
    runtime: WasmRuntime,

    /// Active protocol conversations
    conversations: HashMap<String, Box<dyn ProtocolStateMachine>>,

    /// Incoming message queue
    mailbox: VecDeque<proto::AclMessage>,

    /// Agent capabilities/permissions
    capabilities: proto::AgentCapabilities,

    /// Supervisor address for notifications
    supervisor: Option<Addr<super::Supervisor>>,

    /// Network actor for sending messages
    network: Option<Addr<super::NetworkActor>>,

    /// Registry for looking up other agents
    registry: Option<Addr<super::ActorRegistry>>,

    /// Runtime state
    state: AgentRuntimeState,

    /// Statistics
    stats: AgentStats,

    /// Start time
    start_time: Instant,
}

/// Agent statistics
#[derive(Default)]
struct AgentStats {
    messages_received: u64,
    messages_sent: u64,
    conversations_started: u64,
    conversations_completed: u64,
    errors: u64,
}

impl AgentActor {
    /// Create a new agent actor
    pub fn new(
        config: AgentConfig,
        runtime: WasmRuntime,
    ) -> Self {
        Self {
            agent_id: config.id,
            runtime,
            conversations: HashMap::new(),
            mailbox: VecDeque::new(),
            capabilities: config.capabilities,
            supervisor: None,
            network: None,
            registry: None,
            state: AgentRuntimeState::Starting,
            stats: AgentStats::default(),
            start_time: Instant::now(),
        }
    }

    /// Set the supervisor address
    pub fn with_supervisor(mut self, supervisor: Addr<super::Supervisor>) -> Self {
        self.supervisor = Some(supervisor);
        self
    }

    /// Set the network actor address
    pub fn with_network(mut self, network: Addr<super::NetworkActor>) -> Self {
        self.network = Some(network);
        self
    }

    /// Set the registry address
    pub fn with_registry(mut self, registry: Addr<super::ActorRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Process messages from the mailbox
    fn process_mailbox(&mut self, ctx: &mut Context<Self>) {
        while let Some(msg) = self.mailbox.pop_front() {
            if let Err(e) = self.handle_acl_message(msg, ctx) {
                error!("Error processing message: {}", e);
                self.stats.errors += 1;
            }
        }
    }

    /// Handle a single ACL message
    fn handle_acl_message(
        &mut self,
        msg: proto::AclMessage,
        ctx: &mut Context<Self>,
    ) -> Result<(), AgentError> {
        self.stats.messages_received += 1;

        // Record metrics
        let performative = proto::Performative::try_from(msg.performative)
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|_| "unknown".to_string());
        let protocol = msg.protocol
            .and_then(|p| proto::ProtocolType::try_from(p).ok())
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "unknown".to_string());
        record_message_received(&performative, &protocol);

        // Check if this is part of an existing conversation
        if let Some(conv_id) = &msg.conversation_id {
            if let Some(conversation) = self.conversations.get_mut(conv_id) {
                // Process through protocol state machine
                let result = conversation.process(msg.clone())?;
                self.handle_protocol_result(result, ctx)?;
                return Ok(());
            }
        }

        // New message - pass to WASM module
        let handled = self.runtime.handle_message(&msg)
            .map_err(|e| AgentError::RuntimeError(e.to_string()))?;

        if !handled {
            debug!("Message not handled by agent: {:?}", msg.message_id);
        }

        Ok(())
    }

    /// Handle protocol state machine result
    fn handle_protocol_result(
        &mut self,
        result: crate::protocol::ProcessResult,
        ctx: &mut Context<Self>,
    ) -> Result<(), AgentError> {
        use crate::protocol::ProcessResult;

        match result {
            ProcessResult::Continue => {}
            ProcessResult::Respond(response) => {
                self.send_message(response, ctx)?;
            }
            ProcessResult::Complete(_) => {
                self.stats.conversations_completed += 1;
            }
            ProcessResult::Failed(error) => {
                warn!("Protocol failed: {}", error);
                self.stats.errors += 1;
            }
        }

        Ok(())
    }

    /// Send an ACL message
    fn send_message(
        &mut self,
        msg: proto::AclMessage,
        _ctx: &mut Context<Self>,
    ) -> Result<(), AgentError> {
        if let Some(network) = &self.network {
            // Determine target node
            for _receiver in &msg.receivers {
                // Try local first, then remote
                network.do_send(SendRemoteMessage {
                    target_node: String::new(), // Will be resolved
                    message: msg.clone(),
                });
            }
            self.stats.messages_sent += 1;
        }
        Ok(())
    }

    /// Call the agent's run tick
    fn call_run_tick(&mut self) -> Result<bool, AgentError> {
        self.runtime.call_run()
            .map_err(|e| AgentError::RuntimeError(e.to_string()))
    }

    /// Capture agent state for migration
    fn capture_state(&mut self) -> Result<AgentSnapshot, AgentError> {
        let state = self.runtime.capture_state()
            .map_err(|e| AgentError::RuntimeError(e.to_string()))?;

        let wasm_module = self.runtime.get_module_bytes();
        let mut snapshot = AgentSnapshot {
            agent_id: self.agent_id.clone(),
            wasm_module: wasm_module.to_vec(),
            wasm_hash: [0u8; 32],
            state,
            capabilities: self.capabilities.clone(),
            migration_history: vec![],
        };

        // Compute hash
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&wasm_module);
        snapshot.wasm_hash = hasher.finalize().into();

        Ok(snapshot)
    }

    /// Notify supervisor of an event
    fn notify_supervisor(&self, event: SupervisionEventType) {
        if let Some(supervisor) = &self.supervisor {
            supervisor.do_send(SupervisionEvent {
                agent_id: self.agent_id.clone(),
                event,
            });
        }
    }
}

impl Actor for AgentActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Agent {} starting", self.agent_id.name);
        record_agent_spawned("wasm");

        // Initialize WASM module
        match self.runtime.call_init() {
            Ok(_) => {
                self.state = AgentRuntimeState::Running;
                self.notify_supervisor(SupervisionEventType::Started);
            }
            Err(e) => {
                error!("Agent init failed: {}", e);
                self.state = AgentRuntimeState::Failed;
                self.notify_supervisor(SupervisionEventType::Failed {
                    error: e.to_string(),
                    will_restart: true,
                });
                ctx.stop();
                return;
            }
        }

        // Register with registry
        if let Some(registry) = &self.registry {
            registry.do_send(RegisterActor {
                agent_id: self.agent_id.clone(),
                addr: ctx.address(),
            });
        }

        // Start run loop - tick every 10ms
        ctx.run_interval(Duration::from_millis(10), |actor, ctx| {
            if actor.state != AgentRuntimeState::Running {
                return;
            }

            // Process pending messages
            actor.process_mailbox(ctx);

            // Call agent run tick
            match actor.call_run_tick() {
                Ok(true) => {} // Continue running
                Ok(false) => {
                    // Agent requested stop
                    info!("Agent {} requested stop", actor.agent_id.name);
                    ctx.stop();
                }
                Err(e) => {
                    error!("Agent run error: {}", e);
                    actor.stats.errors += 1;
                }
            }
        });
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        info!("Agent {} stopping", self.agent_id.name);
        record_agent_stopped("wasm", "shutdown");
        self.state = AgentRuntimeState::Stopping;

        // Call shutdown
        if let Err(e) = self.runtime.call_shutdown() {
            warn!("Agent shutdown error: {}", e);
        }

        // Deregister from registry
        if let Some(registry) = &self.registry {
            registry.do_send(DeregisterActor {
                agent_id: self.agent_id.clone(),
            });
        }

        // Notify supervisor
        self.notify_supervisor(SupervisionEventType::Stopped);

        Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Agent {} stopped", self.agent_id.name);
        self.state = AgentRuntimeState::Stopped;
    }
}

// =============================================================================
// Message Handlers
// =============================================================================

impl Handler<DeliverMessage> for AgentActor {
    type Result = Result<(), AgentError>;

    #[instrument(skip(self, msg, _ctx), fields(agent = %self.agent_id.name))]
    fn handle(&mut self, msg: DeliverMessage, _ctx: &mut Self::Context) -> Self::Result {
        // Validate against capabilities
        if let Some(protocol_i32) = msg.message.protocol {
            if !self.capabilities.allowed_protocols.iter().any(|p| *p == protocol_i32) {
                if let Ok(protocol) = proto::ProtocolType::try_from(protocol_i32) {
                    return Err(AgentError::ProtocolNotAllowed(protocol));
                }
            }
        }

        // Queue message for processing
        self.mailbox.push_back(msg.message);
        Ok(())
    }
}

impl Handler<CaptureState> for AgentActor {
    type Result = Result<AgentSnapshot, AgentError>;

    fn handle(&mut self, _msg: CaptureState, _ctx: &mut Self::Context) -> Self::Result {
        self.capture_state()
    }
}

impl Handler<MigrateTo> for AgentActor {
    type Result = ResponseActFuture<Self, Result<(), AgentError>>;

    fn handle(&mut self, msg: MigrateTo, _ctx: &mut Self::Context) -> Self::Result {
        info!("Agent {} migrating to {}", self.agent_id.name, msg.target_node);
        self.state = AgentRuntimeState::Migrating;

        // Capture state
        let snapshot = match self.capture_state() {
            Ok(s) => s,
            Err(e) => {
                self.state = AgentRuntimeState::Running;
                return Box::pin(async move { Err(e) }.into_actor(self));
            }
        };

        let network = self.network.clone();
        let target_node = msg.target_node.clone();
        let reason = msg.reason;

        Box::pin(
            async move {
                if let Some(_network) = network {
                    // Create migration package
                    let _migration = proto::AgentMigration {
                        agent_id: Some(snapshot.agent_id.clone()),
                        wasm_module: Some(snapshot.wasm_module),
                        wasm_hash: snapshot.wasm_hash.to_vec(),
                        state: Some(snapshot.state),
                        capabilities: Some(snapshot.capabilities),
                        migration_history: snapshot.migration_history,
                        reason: proto::MigrationReason::from(reason) as i32,
                        signature: vec![],
                        public_key: vec![],
                        timestamp: chrono::Utc::now().timestamp_millis(),
                    };

                    // Send via network
                    // network.send(...).await
                    Ok(())
                } else {
                    Err(AgentError::NetworkError("No network available".into()))
                }
            }
            .into_actor(self)
            .map(|result, actor, ctx| {
                match &result {
                    Ok(_) => {
                        actor.notify_supervisor(SupervisionEventType::Migrated {
                            from_node: "local".into(),
                            to_node: target_node,
                        });
                        ctx.stop();
                    }
                    Err(_) => {
                        actor.state = AgentRuntimeState::Running;
                    }
                }
                result
            })
        )
    }
}

impl Handler<Shutdown> for AgentActor {
    type Result = ();

    fn handle(&mut self, msg: Shutdown, ctx: &mut Self::Context) {
        info!("Agent {} shutdown requested: {:?}", self.agent_id.name, msg.reason);
        ctx.stop();
    }
}

impl Handler<GetStatus> for AgentActor {
    type Result = AgentStatus;

    fn handle(&mut self, _msg: GetStatus, _ctx: &mut Self::Context) -> Self::Result {
        AgentStatus {
            agent_id: self.agent_id.clone(),
            state: self.state.clone(),
            active_conversations: self.conversations.len(),
            messages_processed: self.stats.messages_received,
            uptime_secs: self.start_time.elapsed().as_secs(),
            memory_used: self.runtime.memory_size(),
        }
    }
}

impl Handler<RegisterService> for AgentActor {
    type Result = Result<(), AgentError>;

    fn handle(&mut self, msg: RegisterService, _ctx: &mut Self::Context) -> Self::Result {
        // Register with the service registry via network
        // For now just log
        info!("Agent {} registering service: {}", self.agent_id.name, msg.service.name);
        Ok(())
    }
}

impl Handler<StartConversation> for AgentActor {
    type Result = Result<String, AgentError>;

    fn handle(&mut self, msg: StartConversation, _ctx: &mut Self::Context) -> Self::Result {
        let conv_id = uuid::Uuid::new_v4().to_string();

        // Create protocol state machine
        let state_machine = crate::protocol::create_state_machine(msg.protocol)
            .map_err(|e| AgentError::InvalidState(e.to_string()))?;

        self.conversations.insert(conv_id.clone(), state_machine);
        self.stats.conversations_started += 1;

        Ok(conv_id)
    }
}
