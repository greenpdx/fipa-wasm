// platform/df.rs - Directory Facilitator (DF)
//
//! Directory Facilitator (DF)
//!
//! The DF is a mandatory FIPA platform agent that provides:
//! - Yellow pages service (agents register/search services)
//! - Multi-criteria search (by name, protocol, ontology, properties)
//! - DF federation (multiple DFs sharing catalogs)
//! - Subscription to DF changes (notify on register/deregister)
//!
//! # FIPA Compliance
//!
//! This implementation follows FIPA00023 (Agent Management Specification).

use actix::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{debug, info};

use crate::proto;

/// DF configuration
#[derive(Debug, Clone)]
pub struct DFConfig {
    /// Platform name
    pub platform_name: String,

    /// Maximum services per agent
    pub max_services_per_agent: usize,

    /// Maximum total services
    pub max_total_services: usize,

    /// Federated DFs
    pub federated_dfs: Vec<String>,
}

impl Default for DFConfig {
    fn default() -> Self {
        Self {
            platform_name: "fipa-platform".to_string(),
            max_services_per_agent: 100,
            max_total_services: 10000,
            federated_dfs: vec![],
        }
    }
}

/// Service registration in DF
#[derive(Debug, Clone)]
pub struct ServiceRegistration {
    /// Service description
    pub service: proto::ServiceDescription,

    /// Owner agent ID
    pub owner: proto::AgentId,

    /// Registration timestamp
    pub registered_at: Instant,

    /// Lease expiry (optional)
    pub lease_until: Option<Instant>,
}

/// Directory Facilitator actor
pub struct DF {
    /// Configuration
    config: DFConfig,

    /// Registered services (service name -> registrations)
    services: HashMap<String, Vec<ServiceRegistration>>,

    /// Services by agent (agent name -> service names)
    agent_services: HashMap<String, HashSet<String>>,

    /// Active subscriptions (subscriber agent -> subscription)
    subscriptions: HashMap<String, DFSubscription>,

    /// Agent ID of this DF
    agent_id: proto::AgentId,

    /// Statistics
    stats: DFStats,
}

/// DF subscription
#[derive(Debug, Clone)]
pub struct DFSubscription {
    /// Subscriber agent ID
    pub subscriber: proto::AgentId,

    /// Filter for notifications
    pub filter: DFSearchFilter,

    /// Created timestamp
    pub created_at: Instant,
}

/// DF statistics
#[derive(Debug, Default, Clone)]
pub struct DFStats {
    pub registrations: u64,
    pub deregistrations: u64,
    pub searches: u64,
    pub search_results: u64,
    pub notifications_sent: u64,
}

impl DF {
    /// Create a new DF
    pub fn new(config: DFConfig) -> Self {
        let platform_name = config.platform_name.clone();
        Self {
            config,
            services: HashMap::new(),
            agent_services: HashMap::new(),
            subscriptions: HashMap::new(),
            agent_id: proto::AgentId {
                name: "df".to_string(),
                addresses: vec![format!("df@{}", platform_name)],
                resolvers: vec![],
            },
            stats: DFStats::default(),
        }
    }

    /// Get the DF agent ID
    pub fn agent_id(&self) -> &proto::AgentId {
        &self.agent_id
    }

    /// Register a service
    fn register(&mut self, request: DFRegister) -> Result<(), DFError> {
        let agent_name = &request.agent_id.name;
        let service_name = &request.service.name;

        // Check agent service limit
        let agent_service_count = self.agent_services
            .get(agent_name)
            .map(|s| s.len())
            .unwrap_or(0);

        if agent_service_count >= self.config.max_services_per_agent {
            return Err(DFError::AgentServiceLimitReached);
        }

        // Check total service limit
        let total_count: usize = self.services.values().map(|v| v.len()).sum();
        if self.config.max_total_services > 0 && total_count >= self.config.max_total_services {
            return Err(DFError::TotalServiceLimitReached);
        }

        // Create registration
        let registration = ServiceRegistration {
            service: request.service.clone(),
            owner: request.agent_id.clone(),
            registered_at: Instant::now(),
            lease_until: request.lease_duration.map(|d| Instant::now() + d),
        };

        // Add to services
        self.services
            .entry(service_name.clone())
            .or_insert_with(Vec::new)
            .push(registration);

        // Track by agent
        self.agent_services
            .entry(agent_name.clone())
            .or_insert_with(HashSet::new)
            .insert(service_name.clone());

        self.stats.registrations += 1;
        info!("DF: Agent '{}' registered service '{}'", agent_name, service_name);

        // Notify subscribers
        self.notify_subscribers(&request.service, DFNotificationType::Registered);

        Ok(())
    }

    /// Deregister a service
    fn deregister(&mut self, request: DFDeregister) -> Result<(), DFError> {
        let agent_name = &request.agent_id.name;
        let service_name = &request.service_name;

        // Find and remove registration
        if let Some(registrations) = self.services.get_mut(service_name) {
            let initial_len = registrations.len();
            registrations.retain(|r| r.owner.name != *agent_name);

            if registrations.len() == initial_len {
                return Err(DFError::ServiceNotFound(service_name.clone()));
            }

            // Clean up empty entries
            if registrations.is_empty() {
                self.services.remove(service_name);
            }

            // Update agent tracking
            if let Some(agent_services) = self.agent_services.get_mut(agent_name) {
                agent_services.remove(service_name);
                if agent_services.is_empty() {
                    self.agent_services.remove(agent_name);
                }
            }

            self.stats.deregistrations += 1;
            info!("DF: Agent '{}' deregistered service '{}'", agent_name, service_name);

            // Notify subscribers
            self.notify_subscribers_deregister(service_name, agent_name);

            Ok(())
        } else {
            Err(DFError::ServiceNotFound(service_name.clone()))
        }
    }

    /// Search for services
    fn search(&mut self, request: DFSearch) -> Vec<DFSearchResult> {
        self.stats.searches += 1;

        let mut results = Vec::new();

        for (_service_name, registrations) in &self.services {
            for reg in registrations {
                if self.matches_filter(&reg.service, &reg.owner, &request.filter) {
                    results.push(DFSearchResult {
                        service: reg.service.clone(),
                        provider: reg.owner.clone(),
                        registered_at: reg.registered_at,
                    });
                }
            }
        }

        // Apply limit
        if let Some(max) = request.max_results {
            results.truncate(max);
        }

        self.stats.search_results += results.len() as u64;
        results
    }

    /// Check if a service matches a filter
    fn matches_filter(
        &self,
        service: &proto::ServiceDescription,
        owner: &proto::AgentId,
        filter: &DFSearchFilter,
    ) -> bool {
        // Name filter
        if let Some(name) = &filter.name {
            if !service.name.contains(name) {
                return false;
            }
        }

        // Protocol filter
        if let Some(protocol) = &filter.protocol {
            let protocol_i32 = *protocol as i32;
            if !service.protocols.contains(&protocol_i32) {
                return false;
            }
        }

        // Ontology filter
        if let Some(ontology) = &filter.ontology {
            if &service.ontology != ontology {
                return false;
            }
        }

        // Owner filter
        if let Some(owner_name) = &filter.owner {
            if &owner.name != owner_name {
                return false;
            }
        }

        // Property filters
        for (key, value) in &filter.properties {
            let matches = service.properties
                .iter()
                .any(|(k, v)| k == key && v == value);
            if !matches {
                return false;
            }
        }

        true
    }

    /// Subscribe to DF changes
    fn subscribe(&mut self, request: DFSubscribe) -> Result<(), DFError> {
        let subscriber_name = request.subscriber.name.clone();

        self.subscriptions.insert(subscriber_name.clone(), DFSubscription {
            subscriber: request.subscriber,
            filter: request.filter,
            created_at: Instant::now(),
        });

        info!("DF: Agent '{}' subscribed to DF changes", subscriber_name);
        Ok(())
    }

    /// Notify subscribers of a registration
    fn notify_subscribers(&mut self, service: &proto::ServiceDescription, notification_type: DFNotificationType) {
        for (_, subscription) in &self.subscriptions {
            // Check if subscriber is interested
            let dummy_owner = proto::AgentId {
                name: String::new(),
                addresses: vec![],
                resolvers: vec![],
            };

            if self.matches_filter(service, &dummy_owner, &subscription.filter) {
                // Would send notification message here
                self.stats.notifications_sent += 1;
                debug!(
                    "DF: Notifying '{}' of {:?} for service '{}'",
                    subscription.subscriber.name, notification_type, service.name
                );
            }
        }
    }

    /// Notify subscribers of a deregistration
    fn notify_subscribers_deregister(&mut self, service_name: &str, _agent_name: &str) {
        for (_, subscription) in &self.subscriptions {
            // Simple name-based check for deregistrations
            if subscription.filter.name.as_ref().map(|n| service_name.contains(n)).unwrap_or(true) {
                self.stats.notifications_sent += 1;
                debug!(
                    "DF: Notifying '{}' of deregistration for service '{}'",
                    subscription.subscriber.name, service_name
                );
            }
        }
    }

    /// Get DF statistics
    pub fn get_stats(&self) -> DFStats {
        self.stats.clone()
    }
}

impl Actor for DF {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        info!("DF started for platform: {}", self.config.platform_name);
    }
}

// =============================================================================
// Messages
// =============================================================================

/// Register a service
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), DFError>")]
pub struct DFRegister {
    /// Agent ID registering the service
    pub agent_id: proto::AgentId,

    /// Service description
    pub service: proto::ServiceDescription,

    /// Optional lease duration
    pub lease_duration: Option<std::time::Duration>,
}

/// Deregister a service
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), DFError>")]
pub struct DFDeregister {
    /// Agent ID deregistering
    pub agent_id: proto::AgentId,

    /// Service name to deregister
    pub service_name: String,
}

/// Modify a service registration
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), DFError>")]
pub struct DFModify {
    /// Agent ID modifying
    pub agent_id: proto::AgentId,

    /// Updated service description
    pub service: proto::ServiceDescription,
}

/// Search for services
#[derive(Debug, Clone, Default, Message)]
#[rtype(result = "Vec<DFSearchResult>")]
pub struct DFSearch {
    /// Search filter
    pub filter: DFSearchFilter,

    /// Maximum results
    pub max_results: Option<usize>,
}

/// Subscribe to DF changes
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), DFError>")]
pub struct DFSubscribe {
    /// Subscriber agent ID
    pub subscriber: proto::AgentId,

    /// Filter for notifications
    pub filter: DFSearchFilter,
}

/// Unsubscribe from DF changes
#[derive(Debug, Clone, Message)]
#[rtype(result = "Result<(), DFError>")]
pub struct DFUnsubscribe {
    /// Subscriber agent ID
    pub subscriber: proto::AgentId,
}

// =============================================================================
// Filter and Result Types
// =============================================================================

/// Search filter for DF queries
#[derive(Debug, Clone, Default)]
pub struct DFSearchFilter {
    /// Service name (partial match)
    pub name: Option<String>,

    /// Required protocol
    pub protocol: Option<proto::ProtocolType>,

    /// Required ontology
    pub ontology: Option<String>,

    /// Owner agent name
    pub owner: Option<String>,

    /// Required properties
    pub properties: HashMap<String, String>,
}

/// Search result from DF
#[derive(Debug, Clone)]
pub struct DFSearchResult {
    /// Service description
    pub service: proto::ServiceDescription,

    /// Service provider agent
    pub provider: proto::AgentId,

    /// Registration timestamp
    pub registered_at: Instant,
}

/// Notification type
#[derive(Debug, Clone)]
pub enum DFNotificationType {
    Registered,
    Deregistered,
    Modified,
}

// =============================================================================
// Errors
// =============================================================================

/// DF errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum DFError {
    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Agent service limit reached")]
    AgentServiceLimitReached,

    #[error("Total service limit reached")]
    TotalServiceLimitReached,

    #[error("Invalid service description: {0}")]
    InvalidService(String),

    #[error("Subscription not found")]
    SubscriptionNotFound,
}

// =============================================================================
// Message Handlers
// =============================================================================

impl Handler<DFRegister> for DF {
    type Result = Result<(), DFError>;

    fn handle(&mut self, msg: DFRegister, _ctx: &mut Self::Context) -> Self::Result {
        self.register(msg)
    }
}

impl Handler<DFDeregister> for DF {
    type Result = Result<(), DFError>;

    fn handle(&mut self, msg: DFDeregister, _ctx: &mut Self::Context) -> Self::Result {
        self.deregister(msg)
    }
}

impl Handler<DFModify> for DF {
    type Result = Result<(), DFError>;

    fn handle(&mut self, msg: DFModify, _ctx: &mut Self::Context) -> Self::Result {
        // Deregister and re-register
        let _ = self.deregister(DFDeregister {
            agent_id: msg.agent_id.clone(),
            service_name: msg.service.name.clone(),
        });

        self.register(DFRegister {
            agent_id: msg.agent_id,
            service: msg.service,
            lease_duration: None,
        })
    }
}

impl Handler<DFSearch> for DF {
    type Result = Vec<DFSearchResult>;

    fn handle(&mut self, msg: DFSearch, _ctx: &mut Self::Context) -> Self::Result {
        self.search(msg)
    }
}

impl Handler<DFSubscribe> for DF {
    type Result = Result<(), DFError>;

    fn handle(&mut self, msg: DFSubscribe, _ctx: &mut Self::Context) -> Self::Result {
        self.subscribe(msg)
    }
}

impl Handler<DFUnsubscribe> for DF {
    type Result = Result<(), DFError>;

    fn handle(&mut self, msg: DFUnsubscribe, _ctx: &mut Self::Context) -> Self::Result {
        if self.subscriptions.remove(&msg.subscriber.name).is_some() {
            info!("DF: Agent '{}' unsubscribed", msg.subscriber.name);
            Ok(())
        } else {
            Err(DFError::SubscriptionNotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_df_config_default() {
        let config = DFConfig::default();
        assert_eq!(config.platform_name, "fipa-platform");
        assert_eq!(config.max_services_per_agent, 100);
    }

    #[test]
    fn test_search_filter_matches() {
        let df = DF::new(DFConfig::default());

        let mut properties = HashMap::new();
        properties.insert("type".to_string(), "basic".to_string());

        let service = proto::ServiceDescription {
            name: "calculator".to_string(),
            description: "Math service".to_string(),
            protocols: vec![proto::ProtocolType::ProtocolQuery as i32],
            ontology: "math".to_string(),
            properties,
        };

        let owner = proto::AgentId {
            name: "calc-agent".to_string(),
            addresses: vec![],
            resolvers: vec![],
        };

        // Empty filter matches everything
        let filter = DFSearchFilter::default();
        assert!(df.matches_filter(&service, &owner, &filter));

        // Name filter
        let filter = DFSearchFilter {
            name: Some("calc".to_string()),
            ..Default::default()
        };
        assert!(df.matches_filter(&service, &owner, &filter));

        // Non-matching name
        let filter = DFSearchFilter {
            name: Some("weather".to_string()),
            ..Default::default()
        };
        assert!(!df.matches_filter(&service, &owner, &filter));
    }
}
