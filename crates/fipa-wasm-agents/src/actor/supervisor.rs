// actor/supervisor.rs - Agent Supervision Tree

use actix::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::actor::messages::*;
use crate::actor::AgentActor;
use crate::content::block::{BlockFile, TAG_DATA, TAG_UNL, TAG_WASM};
use crate::content::unl::{vocabulary_from_bundle, UnlPackager, UnlVerifier, VocabRegistry};
use crate::content::verify::{ContentVerifier, OutboundPackager};
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

    /// Default content verifier applied to spawned agents with no override.
    /// `None` => those agents run without content verification.
    default_verifier: Option<Arc<dyn ContentVerifier>>,

    /// Per-agent verifier overrides, keyed by agent name (each agent ships its
    /// own vocabulary). Takes precedence over the default.
    agent_verifiers: HashMap<String, Arc<dyn ContentVerifier>>,

    /// Shared registry of agents' vocabularies, so the node can check an
    /// outgoing message against the receiver. Populated at spawn from UNL blocks.
    vocab_registry: Arc<RwLock<VocabRegistry>>,

    /// Outbound packager shared by spawned agents (validate + package sends).
    outbound: Arc<dyn OutboundPackager>,

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
        let vocab_registry = Arc::new(RwLock::new(VocabRegistry::new()));
        let outbound: Arc<dyn OutboundPackager> =
            Arc::new(UnlPackager::new(vocab_registry.clone()));
        Self {
            agents: HashMap::new(),
            network: None,
            registry: None,
            default_verifier: None,
            agent_verifiers: HashMap::new(),
            vocab_registry,
            outbound,
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

    /// Set the default content verifier applied to spawned agents.
    pub fn with_content_verifier(mut self, verifier: Arc<dyn ContentVerifier>) -> Self {
        self.default_verifier = Some(verifier);
        self
    }

    /// Override the content verifier for a specific agent (by name) — this is how
    /// each agent ships its own vocabulary. Takes precedence over the default.
    pub fn with_agent_verifier(
        mut self,
        agent_name: impl Into<String>,
        verifier: Arc<dyn ContentVerifier>,
    ) -> Self {
        self.agent_verifiers.insert(agent_name.into(), verifier);
        self
    }

    /// Choose the content verifier for an agent: an operator override, else the
    /// agent's own UNL block (the rules it ships), else the supervisor default.
    fn select_verifier(
        &self,
        agent_name: &str,
        bundle: Option<&BlockFile>,
    ) -> Option<Arc<dyn ContentVerifier>> {
        self.agent_verifiers
            .get(agent_name)
            .cloned()
            .or_else(|| bundle.and_then(Self::verifier_from_bundle))
            .or_else(|| self.default_verifier.clone())
    }

    /// Build a content verifier from an agent bundle's UNL block, if present and
    /// well-formed. A malformed UNL block is logged and skipped (no verifier).
    fn verifier_from_bundle(bundle: &BlockFile) -> Option<Arc<dyn ContentVerifier>> {
        match UnlVerifier::from_bundle(bundle) {
            Some(Ok(v)) => Some(Arc::new(v)),
            Some(Err(e)) => {
                warn!(error = %e, "ignoring malformed UNL block in agent bundle");
                None
            }
            None => None,
        }
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

        // The agent's deployable artifact may be a typed-block bundle (WASM +
        // UNL + ...) or raw WASM (back-compat). The node reads the blocks.
        let bundle = if BlockFile::is_block_container(&config.wasm_module) {
            Some(
                BlockFile::decode(&config.wasm_module)
                    .map_err(|e| AgentError::InvalidState(format!("invalid agent bundle: {e}")))?,
            )
        } else {
            None
        };
        let wasm_bytes: &[u8] = match &bundle {
            Some(b) => b
                .get(TAG_WASM)
                .ok_or_else(|| AgentError::InvalidState("agent bundle has no WASM block".into()))?,
            None => &config.wasm_module,
        };

        // Create WASM runtime
        let mut runtime = WasmRuntime::new(wasm_bytes, &config.capabilities)
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

        // Register the agent's own vocabulary so other agents' outgoing messages
        // can be checked against it (the receiver-side of the send pipeline).
        if let Some(b) = &bundle
            && let Some(Ok(vocab)) = vocabulary_from_bundle(b)
            && let Ok(mut reg) = self.vocab_registry.write()
        {
            reg.register(agent_name.clone(), vocab);
        }

        // Verifier precedence: operator override > the agent's own UNL block
        // (its declared rules) > the supervisor default.
        if let Some(verifier) = self.select_verifier(&agent_name, bundle.as_ref()) {
            actor = actor.with_content_verifier(verifier);
        }

        // The shared outbound packager (validate + package the agent's sends).
        actor = actor.with_outbound_packager(self.outbound.clone());

        // Startup seed: the agent's own UNL + DATA blocks (skipped if absent).
        if let Some(b) = &bundle {
            let seed_unl = b.get(TAG_UNL).map(<[u8]>::to_vec).unwrap_or_default();
            let seed_data = b.get(TAG_DATA).map(<[u8]>::to_vec).unwrap_or_default();
            if !seed_unl.is_empty() || !seed_data.is_empty() {
                actor = actor.with_seed(seed_unl, seed_data);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::block::TAG_UNL;
    use crate::content::unl::{set_unl_content, vocabulary_block};
    use unl_core::{LexCategory, Relation, RelationTag, Uci, Uw, UnlGraph};
    use unl_kb::{ConceptFeatures, Vocabulary};

    // A vocabulary that knows exactly one concept lemma and the `mod` relation.
    fn vocab_knowing(lemma: &str) -> Vocabulary {
        let mut v = Vocabulary::new();
        v.allow_concept(
            1,
            ConceptFeatures { category: LexCategory::Nominal, abstract_: false, gloss: None },
            vec![],
            vec![],
            &[lemma],
        );
        v.allow_relations([RelationTag::Mod]);
        v
    }

    fn verifier_knowing(lemma: &str) -> Arc<dyn ContentVerifier> {
        Arc::new(UnlVerifier::new(vocab_knowing(lemma)))
    }

    fn bundle_knowing(lemma: &str) -> BlockFile {
        BlockFile::new().with(TAG_UNL, vocabulary_block(&vocab_knowing(lemma)))
    }

    fn empty_msg() -> proto::AclMessage {
        proto::AclMessage {
            message_id: "m".into(),
            performative: proto::Performative::Inform as i32,
            sender: Some(proto::AgentId { name: "s".into(), addresses: vec![], resolvers: vec![] }),
            receivers: vec![],
            reply_to: None,
            protocol: None,
            conversation_id: None,
            in_reply_to: None,
            reply_with: None,
            reply_by: None,
            language: None,
            encoding: None,
            ontology: None,
            content: vec![],
            user_properties: HashMap::new(),
        }
    }

    // A message whose UNL content is `mod(lemma, lemma)`.
    fn msg_about(lemma: &str) -> proto::AclMessage {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn(lemma)));
        g.insert_node("02", Uw::new(Uci::ucn(lemma)));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Mod, "01".into(), "02".into()));
        let mut m = empty_msg();
        set_unl_content(&mut m, &g);
        m
    }

    #[test]
    fn verifier_precedence_override_then_bundle_then_default() {
        // override→cat, bundle→dog, default→fish — three distinguishable vocabs.
        let sup = Supervisor::new("n".into())
            .with_content_verifier(verifier_knowing("fish"))
            .with_agent_verifier("a1", verifier_knowing("cat"));
        let bundle = bundle_knowing("dog");

        // a1: operator override (cat) wins over bundle(dog) and default(fish).
        let v = sup.select_verifier("a1", Some(&bundle)).unwrap();
        assert!(v.verify(&msg_about("cat")).is_ok());
        assert!(v.verify(&msg_about("dog")).is_err());

        // a2: no override → the agent's own UNL block (dog) beats default(fish).
        let v = sup.select_verifier("a2", Some(&bundle)).unwrap();
        assert!(v.verify(&msg_about("dog")).is_ok());
        assert!(v.verify(&msg_about("fish")).is_err());

        // a3: no override, no bundle → supervisor default (fish).
        let v = sup.select_verifier("a3", None).unwrap();
        assert!(v.verify(&msg_about("fish")).is_ok());
    }

    #[test]
    fn no_sources_means_no_verifier() {
        let sup = Supervisor::new("n".into());
        assert!(sup.select_verifier("a", None).is_none());
    }
}
