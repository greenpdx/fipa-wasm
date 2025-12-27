// network/routing.rs - Message Routing Actor

use actix::prelude::*;
use libp2p::{Multiaddr, PeerId};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::actor::{
    AgentError, DeliverMessage, NodeDiscovered, NodeDisconnected, SendRemoteMessage,
};
use crate::proto;
use super::transport::{NetworkConfig, NetworkTransport};
use super::discovery::{DiscoveryConfig, DiscoveryService, DiscoverySource};

/// Message to route to a peer
#[derive(Message, Clone)]
#[rtype(result = "Result<(), crate::actor::AgentError>")]
pub struct RouteMessage {
    pub target_node: String,
    pub envelope: proto::MessageEnvelope,
}

/// Connect to a peer
#[derive(Message)]
#[rtype(result = "Result<(), crate::actor::AgentError>")]
pub struct ConnectPeer {
    pub peer_id: String,
    pub addresses: Vec<String>,
}

/// Disconnect from a peer
#[derive(Message)]
#[rtype(result = "()")]
pub struct DisconnectPeer {
    pub peer_id: String,
}

/// Get network status
#[derive(Message)]
#[rtype(result = "NetworkStatus")]
pub struct GetNetworkStatus;

/// Network status
#[derive(Debug, Clone)]
pub struct NetworkStatus {
    pub local_peer_id: String,
    pub listen_addresses: Vec<String>,
    pub connected_peers: usize,
    pub known_peers: usize,
    pub messages_sent: u64,
    pub messages_received: u64,
}

impl<A, M> actix::dev::MessageResponse<A, M> for NetworkStatus
where
    A: actix::Actor,
    M: actix::Message<Result = NetworkStatus>,
{
    fn handle(self, _ctx: &mut A::Context, tx: Option<actix::dev::OneshotSender<M::Result>>) {
        if let Some(tx) = tx {
            let _ = tx.send(self);
        }
    }
}

/// Network actor handling all network operations
pub struct NetworkActor {
    /// Node ID
    node_id: String,

    /// Transport layer
    transport: NetworkTransport,

    /// Discovery service
    discovery: DiscoveryService,

    /// Supervisor for agent messages
    supervisor: Option<Addr<crate::actor::Supervisor>>,

    /// Registry for agent lookups
    registry: Option<Addr<crate::actor::ActorRegistry>>,

    /// Pending outbound messages (node_id -> messages)
    pending_outbound: HashMap<String, Vec<proto::MessageEnvelope>>,

    /// Statistics
    messages_sent: u64,
    messages_received: u64,
}

impl NetworkActor {
    /// Create new network actor
    pub fn new(node_id: String, config: NetworkConfig) -> Self {
        let transport = NetworkTransport::new(config);
        let discovery = DiscoveryService::new(DiscoveryConfig::default());

        Self {
            node_id,
            transport,
            discovery,
            supervisor: None,
            registry: None,
            pending_outbound: HashMap::new(),
            messages_sent: 0,
            messages_received: 0,
        }
    }

    /// Set supervisor
    pub fn with_supervisor(mut self, supervisor: Addr<crate::actor::Supervisor>) -> Self {
        self.supervisor = Some(supervisor);
        self
    }

    /// Set registry
    pub fn with_registry(mut self, registry: Addr<crate::actor::ActorRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Route a message to target node
    fn route_message(&mut self, msg: RouteMessage) -> Result<(), AgentError> {
        let target = &msg.target_node;

        // Check if we have a connection to target
        // For now, queue the message
        self.pending_outbound
            .entry(target.clone())
            .or_insert_with(Vec::new)
            .push(msg.envelope);

        // In real implementation, would send via libp2p
        self.messages_sent += 1;
        debug!("Routed message to {}", target);

        Ok(())
    }

    /// Handle incoming message from network
    #[allow(dead_code)]
    fn handle_incoming(&mut self, envelope: proto::MessageEnvelope, _ctx: &mut Context<Self>) {
        self.messages_received += 1;

        match &envelope.payload {
            Some(proto::message_envelope::Payload::AclMessage(msg)) => {
                // Deliver to local agent
                if let Some(supervisor) = &self.supervisor {
                    supervisor.do_send(DeliverMessage {
                        message: msg.clone(),
                    });
                }
            }
            Some(proto::message_envelope::Payload::Migration(migration)) => {
                // Handle agent migration
                info!("Received agent migration: {:?}", migration.agent_id);
                // Would spawn the migrated agent
            }
            Some(proto::message_envelope::Payload::RegistryUpdate(_update)) => {
                // Handle registry update
                if let Some(_registry) = &self.registry {
                    // Forward to registry
                }
            }
            Some(proto::message_envelope::Payload::Consensus(_consensus)) => {
                // Handle consensus message
                // Would forward to Raft
            }
            Some(proto::message_envelope::Payload::HealthPing(ping)) => {
                // Handle health ping
                debug!("Health ping from {}", ping.node_id);
            }
            None => {
                warn!("Received envelope with no payload");
            }
        }
    }
}

impl Actor for NetworkActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("NetworkActor started for node: {}", self.node_id);

        // Start periodic cleanup
        ctx.run_interval(std::time::Duration::from_secs(60), |actor, _ctx| {
            actor.discovery.cleanup_stale();
        });
    }
}

impl Handler<RouteMessage> for NetworkActor {
    type Result = Result<(), AgentError>;

    fn handle(&mut self, msg: RouteMessage, _ctx: &mut Self::Context) -> Self::Result {
        self.route_message(msg)
    }
}

impl Handler<SendRemoteMessage> for NetworkActor {
    type Result = Result<String, AgentError>;

    fn handle(&mut self, msg: SendRemoteMessage, _ctx: &mut Self::Context) -> Self::Result {
        let message_id = uuid::Uuid::new_v4().to_string();

        let envelope = proto::MessageEnvelope {
            source_node: self.node_id.clone(),
            target_node: msg.target_node.clone(),
            sequence: self.messages_sent,
            timestamp: chrono::Utc::now().timestamp_millis(),
            payload: Some(proto::message_envelope::Payload::AclMessage(msg.message)),
        };

        self.route_message(RouteMessage {
            target_node: msg.target_node,
            envelope,
        })?;

        Ok(message_id)
    }
}

impl Handler<ConnectPeer> for NetworkActor {
    type Result = Result<(), AgentError>;

    fn handle(&mut self, msg: ConnectPeer, _ctx: &mut Self::Context) -> Self::Result {
        info!("Connecting to peer: {}", msg.peer_id);

        // Parse addresses
        for addr_str in &msg.addresses {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                if let Ok(peer_id) = msg.peer_id.parse::<PeerId>() {
                    self.discovery.add_peer(peer_id, addr, DiscoverySource::Direct);
                }
            }
        }

        Ok(())
    }
}

impl Handler<DisconnectPeer> for NetworkActor {
    type Result = ();

    fn handle(&mut self, msg: DisconnectPeer, _ctx: &mut Self::Context) {
        info!("Disconnecting from peer: {}", msg.peer_id);

        if let Ok(peer_id) = msg.peer_id.parse::<PeerId>() {
            self.discovery.remove_peer(&peer_id);
        }
    }
}

impl Handler<GetNetworkStatus> for NetworkActor {
    type Result = NetworkStatus;

    fn handle(&mut self, _msg: GetNetworkStatus, _ctx: &mut Self::Context) -> Self::Result {
        NetworkStatus {
            local_peer_id: self.transport.peer_id().to_string(),
            listen_addresses: vec![], // Would get from transport
            connected_peers: self.discovery.peer_count(),
            known_peers: self.discovery.peer_count(),
            messages_sent: self.messages_sent,
            messages_received: self.messages_received,
        }
    }
}

impl Handler<NodeDiscovered> for NetworkActor {
    type Result = ();

    fn handle(&mut self, msg: NodeDiscovered, _ctx: &mut Self::Context) {
        info!("Node discovered: {}", msg.node_id);

        // Parse and add addresses
        for addr_str in &msg.addresses {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                if let Ok(peer_id) = msg.node_id.parse::<PeerId>() {
                    self.discovery.add_peer(peer_id, addr, DiscoverySource::Mdns);
                }
            }
        }
    }
}

impl Handler<NodeDisconnected> for NetworkActor {
    type Result = ();

    fn handle(&mut self, msg: NodeDisconnected, _ctx: &mut Self::Context) {
        info!("Node disconnected: {}", msg.node_id);

        if let Ok(peer_id) = msg.node_id.parse::<PeerId>() {
            self.discovery.remove_peer(&peer_id);
        }
    }
}
