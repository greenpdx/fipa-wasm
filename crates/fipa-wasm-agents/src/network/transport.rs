// network/transport.rs - libp2p Transport Configuration

use anyhow::Result;
use libp2p::{
    identity::Keypair,
    Multiaddr, PeerId,
};
use std::time::Duration;
use tracing::info;

/// Network configuration
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Node keypair for identity
    pub keypair: Keypair,

    /// Listen addresses
    pub listen_addrs: Vec<Multiaddr>,

    /// Bootstrap peers
    pub bootstrap_peers: Vec<(PeerId, Multiaddr)>,

    /// Enable mDNS discovery
    pub enable_mdns: bool,

    /// Enable Kademlia DHT
    pub enable_kademlia: bool,

    /// Connection timeout
    pub connection_timeout: Duration,

    /// Max concurrent connections
    pub max_connections: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            keypair: Keypair::generate_ed25519(),
            listen_addrs: vec![
                "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
            ],
            bootstrap_peers: vec![],
            enable_mdns: true,
            enable_kademlia: true,
            connection_timeout: Duration::from_secs(30),
            max_connections: 100,
        }
    }
}

impl NetworkConfig {
    /// Create config with specific keypair
    pub fn with_keypair(mut self, keypair: Keypair) -> Self {
        self.keypair = keypair;
        self
    }

    /// Add listen address
    pub fn with_listen_addr(mut self, addr: Multiaddr) -> Self {
        self.listen_addrs.push(addr);
        self
    }

    /// Add bootstrap peer
    pub fn with_bootstrap_peer(mut self, peer_id: PeerId, addr: Multiaddr) -> Self {
        self.bootstrap_peers.push((peer_id, addr));
        self
    }

    /// Get local peer ID
    pub fn peer_id(&self) -> PeerId {
        PeerId::from(self.keypair.public())
    }
}

/// Swarm event types we handle
#[derive(Debug)]
pub enum SwarmEvent {
    /// New peer discovered
    PeerDiscovered(PeerId),

    /// Peer disconnected
    PeerDisconnected(PeerId),

    /// Peer address added
    PeerAddressAdded(PeerId, Multiaddr),

    /// Listening on address
    Listening(Multiaddr),

    /// Incoming message
    IncomingMessage {
        peer: PeerId,
        data: Vec<u8>,
    },

    /// Message sent successfully
    MessageSent {
        peer: PeerId,
        request_id: u64,
    },
}

/// Network transport wrapper
pub struct NetworkTransport {
    /// Configuration
    #[allow(dead_code)]
    config: NetworkConfig,

    /// Local peer ID
    peer_id: PeerId,

    /// Known peers
    peers: std::collections::HashMap<PeerId, Vec<Multiaddr>>,

    /// Pending requests
    pending_requests: std::collections::HashMap<u64, PeerId>,

    /// Next request ID
    next_request_id: u64,
}

impl NetworkTransport {
    /// Create new transport
    pub fn new(config: NetworkConfig) -> Self {
        let peer_id = config.peer_id();
        info!("Created network transport with peer ID: {}", peer_id);

        Self {
            config,
            peer_id,
            peers: std::collections::HashMap::new(),
            pending_requests: std::collections::HashMap::new(),
            next_request_id: 1,
        }
    }

    /// Get local peer ID
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Get known peers
    pub fn peers(&self) -> impl Iterator<Item = &PeerId> {
        self.peers.keys()
    }

    /// Add a peer address
    pub fn add_peer(&mut self, peer_id: PeerId, addr: Multiaddr) {
        self.peers
            .entry(peer_id)
            .or_insert_with(Vec::new)
            .push(addr);
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, peer_id: &PeerId) {
        self.peers.remove(peer_id);
    }

    /// Get addresses for a peer
    pub fn get_peer_addrs(&self, peer_id: &PeerId) -> Option<&Vec<Multiaddr>> {
        self.peers.get(peer_id)
    }

    /// Allocate a new request ID
    pub fn next_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    /// Track a pending request
    pub fn track_request(&mut self, request_id: u64, peer: PeerId) {
        self.pending_requests.insert(request_id, peer);
    }

    /// Complete a pending request
    pub fn complete_request(&mut self, request_id: u64) -> Option<PeerId> {
        self.pending_requests.remove(&request_id)
    }
}

/// Create the libp2p swarm (placeholder - full implementation would be more complex)
#[allow(dead_code)]
pub async fn create_swarm(config: &NetworkConfig) -> Result<()> {
    // In a full implementation, this would create the libp2p swarm
    // with all the configured protocols
    info!("Creating libp2p swarm with peer ID: {}", config.peer_id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_config_default() {
        let config = NetworkConfig::default();
        assert!(config.enable_mdns);
        assert!(config.enable_kademlia);
        assert!(!config.listen_addrs.is_empty());
    }

    #[test]
    fn test_network_transport() {
        let config = NetworkConfig::default();
        let mut transport = NetworkTransport::new(config);

        let peer_id = PeerId::random();
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();

        transport.add_peer(peer_id, addr.clone());
        assert!(transport.get_peer_addrs(&peer_id).is_some());

        transport.remove_peer(&peer_id);
        assert!(transport.get_peer_addrs(&peer_id).is_none());
    }
}
