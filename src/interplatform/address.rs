// interplatform/address.rs - Platform Address Resolution
//
//! Agent and Platform Address Resolution
//!
//! Provides address parsing, resolution, and platform discovery.

use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Address resolution errors
#[derive(Debug, Error)]
pub enum AddressError {
    #[error("Invalid address format: {0}")]
    InvalidFormat(String),

    #[error("Unknown platform: {0}")]
    UnknownPlatform(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Resolution failed: {0}")]
    ResolutionFailed(String),

    #[error("No transport address available")]
    NoTransportAddress,
}

/// Agent address (name@platform format)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentAddress {
    /// Agent local name
    pub name: String,

    /// Platform identifier
    pub platform: Option<String>,

    /// Transport addresses (URLs)
    pub addresses: Vec<String>,

    /// Name resolution service addresses
    pub resolvers: Vec<String>,
}

impl AgentAddress {
    /// Create a local agent address
    pub fn local(name: &str) -> Self {
        Self {
            name: name.to_string(),
            platform: None,
            addresses: vec![],
            resolvers: vec![],
        }
    }

    /// Create a remote agent address
    pub fn remote(name: &str, platform: &str) -> Self {
        Self {
            name: name.to_string(),
            platform: Some(platform.to_string()),
            addresses: vec![],
            resolvers: vec![],
        }
    }

    /// Parse from string (formats: "name", "name@platform", "name@http://platform.com")
    pub fn parse(s: &str) -> Result<Self, AddressError> {
        let s = s.trim();

        if s.is_empty() {
            return Err(AddressError::InvalidFormat("Empty address".to_string()));
        }

        // Check for @ separator
        if let Some(pos) = s.find('@') {
            let name = &s[..pos];
            let rest = &s[pos + 1..];

            if name.is_empty() {
                return Err(AddressError::InvalidFormat(
                    "Empty agent name".to_string(),
                ));
            }

            // Check if rest is a URL or platform name
            let (platform, addresses) = if rest.contains("://") {
                // It's a URL
                (None, vec![rest.to_string()])
            } else {
                // It's a platform name
                (Some(rest.to_string()), vec![])
            };

            Ok(Self {
                name: name.to_string(),
                platform,
                addresses,
                resolvers: vec![],
            })
        } else {
            // Just an agent name (local)
            Ok(Self::local(s))
        }
    }

    /// Add a transport address
    pub fn with_address(mut self, addr: &str) -> Self {
        self.addresses.push(addr.to_string());
        self
    }

    /// Add a resolver
    pub fn with_resolver(mut self, resolver: &str) -> Self {
        self.resolvers.push(resolver.to_string());
        self
    }

    /// Check if this is a local address (no platform specified)
    pub fn is_local(&self) -> bool {
        self.platform.is_none() && self.addresses.is_empty()
    }

    /// Check if this is a remote address
    pub fn is_remote(&self) -> bool {
        self.platform.is_some() || !self.addresses.is_empty()
    }

    /// Get the fully qualified name
    pub fn qualified_name(&self) -> String {
        if let Some(ref platform) = self.platform {
            format!("{}@{}", self.name, platform)
        } else if let Some(addr) = self.addresses.first() {
            format!("{}@{}", self.name, addr)
        } else {
            self.name.clone()
        }
    }

    /// Get the primary transport address
    pub fn primary_address(&self) -> Option<&String> {
        self.addresses.first()
    }
}

impl std::fmt::Display for AgentAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.qualified_name())
    }
}

/// Platform address (transport endpoints)
#[derive(Debug, Clone)]
pub struct PlatformAddress {
    /// Platform name/identifier
    pub name: String,

    /// Platform description
    pub description: Option<String>,

    /// HTTP transport addresses
    pub http_addresses: Vec<String>,

    /// gRPC transport addresses
    pub grpc_addresses: Vec<String>,

    /// Other transport addresses
    pub other_addresses: HashMap<String, Vec<String>>,

    /// AMS address (for agent management)
    pub ams_address: Option<String>,

    /// DF address (for service discovery)
    pub df_address: Option<String>,

    /// Last seen timestamp
    pub last_seen: u64,

    /// Is this the local platform
    pub is_local: bool,
}

impl PlatformAddress {
    /// Create a new platform address
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            description: None,
            http_addresses: vec![],
            grpc_addresses: vec![],
            other_addresses: HashMap::new(),
            ams_address: None,
            df_address: None,
            last_seen: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            is_local: false,
        }
    }

    /// Mark as local platform
    pub fn local(mut self) -> Self {
        self.is_local = true;
        self
    }

    /// Add an HTTP address
    pub fn with_http(mut self, addr: &str) -> Self {
        self.http_addresses.push(addr.to_string());
        self
    }

    /// Add a gRPC address
    pub fn with_grpc(mut self, addr: &str) -> Self {
        self.grpc_addresses.push(addr.to_string());
        self
    }

    /// Set AMS address
    pub fn with_ams(mut self, addr: &str) -> Self {
        self.ams_address = Some(addr.to_string());
        self
    }

    /// Set DF address
    pub fn with_df(mut self, addr: &str) -> Self {
        self.df_address = Some(addr.to_string());
        self
    }

    /// Get all transport addresses
    pub fn all_addresses(&self) -> Vec<&String> {
        let mut addrs: Vec<&String> = self.http_addresses.iter().collect();
        addrs.extend(self.grpc_addresses.iter());
        for addresses in self.other_addresses.values() {
            addrs.extend(addresses.iter());
        }
        addrs
    }

    /// Get primary address (prefers HTTP)
    pub fn primary_address(&self) -> Option<&String> {
        self.http_addresses
            .first()
            .or_else(|| self.grpc_addresses.first())
    }

    /// Update last seen
    pub fn touch(&mut self) {
        self.last_seen = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }
}

/// Address resolver - resolves agent and platform addresses
pub struct AddressResolver {
    /// Known platforms
    platforms: Arc<RwLock<HashMap<String, PlatformAddress>>>,

    /// Agent location cache (agent -> platform)
    agent_cache: Arc<RwLock<HashMap<String, String>>>,

    /// Local platform name
    local_platform: String,

    /// Cache TTL in seconds
    cache_ttl_secs: u64,
}

impl AddressResolver {
    /// Create a new resolver
    pub fn new(local_platform: &str) -> Self {
        Self {
            platforms: Arc::new(RwLock::new(HashMap::new())),
            agent_cache: Arc::new(RwLock::new(HashMap::new())),
            local_platform: local_platform.to_string(),
            cache_ttl_secs: 300, // 5 minutes
        }
    }

    /// Set cache TTL
    pub fn with_cache_ttl(mut self, ttl_secs: u64) -> Self {
        self.cache_ttl_secs = ttl_secs;
        self
    }

    /// Register the local platform
    pub async fn register_local(&self, platform: PlatformAddress) {
        let mut platforms = self.platforms.write().await;
        let mut local = platform;
        local.is_local = true;
        platforms.insert(self.local_platform.clone(), local);
    }

    /// Register a remote platform
    pub async fn register_platform(&self, platform: PlatformAddress) {
        let mut platforms = self.platforms.write().await;
        platforms.insert(platform.name.clone(), platform);
    }

    /// Unregister a platform
    pub async fn unregister_platform(&self, name: &str) -> bool {
        let mut platforms = self.platforms.write().await;
        platforms.remove(name).is_some()
    }

    /// Get a platform by name
    pub async fn get_platform(&self, name: &str) -> Option<PlatformAddress> {
        let platforms = self.platforms.read().await;
        platforms.get(name).cloned()
    }

    /// List all known platforms
    pub async fn list_platforms(&self) -> Vec<PlatformAddress> {
        let platforms = self.platforms.read().await;
        platforms.values().cloned().collect()
    }

    /// Register agent location
    pub async fn register_agent(&self, agent: &str, platform: &str) {
        let mut cache = self.agent_cache.write().await;
        cache.insert(agent.to_string(), platform.to_string());
    }

    /// Find agent's platform
    pub async fn find_agent(&self, agent: &str) -> Option<String> {
        let cache = self.agent_cache.read().await;
        cache.get(agent).cloned()
    }

    /// Resolve an agent address to transport addresses
    pub async fn resolve(&self, address: &AgentAddress) -> Result<Vec<String>, AddressError> {
        // If already has transport addresses, return them
        if !address.addresses.is_empty() {
            return Ok(address.addresses.clone());
        }

        // Check if it's a local agent
        if address.is_local() {
            // Local agents don't need transport addresses
            return Ok(vec![]);
        }

        // Try to resolve platform
        if let Some(ref platform_name) = address.platform {
            let platforms = self.platforms.read().await;
            if let Some(platform) = platforms.get(platform_name) {
                if let Some(addr) = platform.primary_address() {
                    return Ok(vec![addr.clone()]);
                }
            }

            return Err(AddressError::UnknownPlatform(platform_name.clone()));
        }

        // Check agent cache
        if let Some(platform_name) = self.find_agent(&address.name).await {
            let platforms = self.platforms.read().await;
            if let Some(platform) = platforms.get(&platform_name) {
                if let Some(addr) = platform.primary_address() {
                    return Ok(vec![addr.clone()]);
                }
            }
        }

        Err(AddressError::AgentNotFound(address.name.clone()))
    }

    /// Check if an address is local
    pub fn is_local(&self, address: &AgentAddress) -> bool {
        if address.is_local() {
            return true;
        }

        if let Some(ref platform) = address.platform {
            platform == &self.local_platform
        } else {
            false
        }
    }

    /// Get the local platform name
    pub fn local_platform(&self) -> &str {
        &self.local_platform
    }

    /// Clear expired cache entries
    pub async fn cleanup_cache(&self) {
        // In a real implementation, we would check timestamps
        // For now, this is a placeholder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_address_local() {
        let addr = AgentAddress::local("my-agent");
        assert_eq!(addr.name, "my-agent");
        assert!(addr.is_local());
        assert!(!addr.is_remote());
    }

    #[test]
    fn test_agent_address_remote() {
        let addr = AgentAddress::remote("my-agent", "other-platform");
        assert_eq!(addr.name, "my-agent");
        assert!(addr.is_remote());
        assert!(!addr.is_local());
        assert_eq!(addr.qualified_name(), "my-agent@other-platform");
    }

    #[test]
    fn test_agent_address_parse() {
        // Local name only
        let addr = AgentAddress::parse("agent1").unwrap();
        assert_eq!(addr.name, "agent1");
        assert!(addr.is_local());

        // Name@platform
        let addr = AgentAddress::parse("agent1@platform1").unwrap();
        assert_eq!(addr.name, "agent1");
        assert_eq!(addr.platform, Some("platform1".to_string()));

        // Name@URL
        let addr = AgentAddress::parse("agent1@http://platform.example.com").unwrap();
        assert_eq!(addr.name, "agent1");
        assert_eq!(addr.addresses, vec!["http://platform.example.com"]);
    }

    #[test]
    fn test_agent_address_parse_errors() {
        assert!(AgentAddress::parse("").is_err());
        assert!(AgentAddress::parse("@platform").is_err());
    }

    #[test]
    fn test_platform_address() {
        let platform = PlatformAddress::new("test-platform")
            .with_http("http://platform.example.com:8080")
            .with_grpc("grpc://platform.example.com:9090")
            .with_ams("ams@http://platform.example.com:8080")
            .with_df("df@http://platform.example.com:8080");

        assert_eq!(platform.name, "test-platform");
        assert_eq!(platform.http_addresses.len(), 1);
        assert_eq!(platform.grpc_addresses.len(), 1);
        assert!(platform.ams_address.is_some());
    }

    #[test]
    fn test_platform_all_addresses() {
        let platform = PlatformAddress::new("test")
            .with_http("http://a.com")
            .with_http("http://b.com")
            .with_grpc("grpc://c.com");

        let addrs = platform.all_addresses();
        assert_eq!(addrs.len(), 3);
    }

    #[tokio::test]
    async fn test_address_resolver() {
        let resolver = AddressResolver::new("local-platform");

        // Register local platform
        let local = PlatformAddress::new("local-platform")
            .with_http("http://localhost:8080")
            .local();
        resolver.register_local(local).await;

        // Register remote platform
        let remote = PlatformAddress::new("remote-platform")
            .with_http("http://remote.example.com:8080");
        resolver.register_platform(remote).await;

        // List platforms
        let platforms = resolver.list_platforms().await;
        assert_eq!(platforms.len(), 2);

        // Resolve remote agent
        let addr = AgentAddress::remote("agent1", "remote-platform");
        let resolved = resolver.resolve(&addr).await.unwrap();
        assert_eq!(resolved, vec!["http://remote.example.com:8080"]);
    }

    #[tokio::test]
    async fn test_resolver_is_local() {
        let resolver = AddressResolver::new("my-platform");

        assert!(resolver.is_local(&AgentAddress::local("agent1")));
        assert!(resolver.is_local(&AgentAddress::remote("agent1", "my-platform")));
        assert!(!resolver.is_local(&AgentAddress::remote("agent1", "other-platform")));
    }

    #[tokio::test]
    async fn test_agent_cache() {
        let resolver = AddressResolver::new("local");

        // Register platform
        let platform = PlatformAddress::new("platform1")
            .with_http("http://platform1.example.com");
        resolver.register_platform(platform).await;

        // Register agent location
        resolver.register_agent("agent1", "platform1").await;

        // Find agent
        let found = resolver.find_agent("agent1").await;
        assert_eq!(found, Some("platform1".to_string()));

        // Resolve agent address
        let addr = AgentAddress::local("agent1");
        let resolved = resolver.resolve(&addr).await;
        // Should fail because local agent doesn't need resolution
        assert!(resolved.is_ok());
        assert!(resolved.unwrap().is_empty());
    }
}
