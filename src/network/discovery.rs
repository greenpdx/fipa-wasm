// network/discovery.rs - Peer Discovery Service

use libp2p::{Multiaddr, PeerId};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Discovery configuration
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Enable mDNS for local discovery
    pub enable_mdns: bool,

    /// Enable Kademlia DHT for wide-area discovery
    pub enable_kademlia: bool,

    /// Bootstrap peers for Kademlia
    pub bootstrap_peers: Vec<(PeerId, Multiaddr)>,

    /// How often to refresh DHT
    pub dht_refresh_interval: Duration,

    /// How long to cache peer information
    pub peer_cache_ttl: Duration,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enable_mdns: true,
            enable_kademlia: true,
            bootstrap_peers: vec![],
            dht_refresh_interval: Duration::from_secs(300),
            peer_cache_ttl: Duration::from_secs(600),
        }
    }
}

/// Discovered peer information
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    /// Peer ID
    pub peer_id: PeerId,

    /// Known addresses
    pub addresses: Vec<Multiaddr>,

    /// Discovery source
    pub source: DiscoverySource,

    /// When discovered
    pub discovered_at: Instant,

    /// Last seen
    pub last_seen: Instant,
}

/// How a peer was discovered
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverySource {
    /// Local network mDNS
    Mdns,

    /// Kademlia DHT
    Kademlia,

    /// Bootstrap configuration
    Bootstrap,

    /// Direct connection
    Direct,

    /// Peer exchange
    PeerExchange,
}

/// Discovery service for finding peers
pub struct DiscoveryService {
    /// Configuration
    config: DiscoveryConfig,

    /// Discovered peers
    peers: HashMap<PeerId, DiscoveredPeer>,

    /// Service registry (service_name -> peer_ids)
    services: HashMap<String, Vec<PeerId>>,
}

impl DiscoveryService {
    /// Create new discovery service
    pub fn new(config: DiscoveryConfig) -> Self {
        Self {
            config,
            peers: HashMap::new(),
            services: HashMap::new(),
        }
    }

    /// Add a discovered peer
    pub fn add_peer(&mut self, peer_id: PeerId, addr: Multiaddr, source: DiscoverySource) {
        let now = Instant::now();

        if let Some(peer) = self.peers.get_mut(&peer_id) {
            // Update existing peer
            if !peer.addresses.contains(&addr) {
                peer.addresses.push(addr);
            }
            peer.last_seen = now;
            debug!("Updated peer: {}", peer_id);
        } else {
            // New peer
            let peer = DiscoveredPeer {
                peer_id,
                addresses: vec![addr],
                source,
                discovered_at: now,
                last_seen: now,
            };
            self.peers.insert(peer_id, peer);
            info!("Discovered new peer: {}", peer_id);
        }
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, peer_id: &PeerId) {
        self.peers.remove(peer_id);

        // Remove from services
        for (_, peers) in self.services.iter_mut() {
            peers.retain(|p| p != peer_id);
        }
    }

    /// Get a peer
    pub fn get_peer(&self, peer_id: &PeerId) -> Option<&DiscoveredPeer> {
        self.peers.get(peer_id)
    }

    /// Get all peers
    pub fn all_peers(&self) -> impl Iterator<Item = &DiscoveredPeer> {
        self.peers.values()
    }

    /// Get peers by discovery source
    pub fn peers_by_source(&self, source: DiscoverySource) -> impl Iterator<Item = &DiscoveredPeer> {
        self.peers.values().filter(move |p| p.source == source)
    }

    /// Register a service provider
    pub fn register_service(&mut self, service_name: String, peer_id: PeerId) {
        self.services
            .entry(service_name)
            .or_insert_with(Vec::new)
            .push(peer_id);
    }

    /// Find service providers
    pub fn find_service(&self, service_name: &str) -> Vec<&DiscoveredPeer> {
        self.services
            .get(service_name)
            .map(|peers| {
                peers
                    .iter()
                    .filter_map(|p| self.peers.get(p))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clean up stale peers
    pub fn cleanup_stale(&mut self) {
        let ttl = self.config.peer_cache_ttl;
        let now = Instant::now();

        self.peers.retain(|_, peer| {
            now.duration_since(peer.last_seen) < ttl
        });
    }

    /// Get peer count
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_service() {
        let config = DiscoveryConfig::default();
        let mut service = DiscoveryService::new(config);

        let peer_id = PeerId::random();
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();

        service.add_peer(peer_id, addr, DiscoverySource::Mdns);
        assert_eq!(service.peer_count(), 1);

        let peer = service.get_peer(&peer_id).unwrap();
        assert_eq!(peer.source, DiscoverySource::Mdns);
    }

    #[test]
    fn test_service_registry() {
        let config = DiscoveryConfig::default();
        let mut service = DiscoveryService::new(config);

        let peer_id = PeerId::random();
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();

        service.add_peer(peer_id, addr, DiscoverySource::Direct);
        service.register_service("data-processing".into(), peer_id);

        let providers = service.find_service("data-processing");
        assert_eq!(providers.len(), 1);
    }
}
