// consensus/state.rs - Cluster State Types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use openraft::{LogId, StoredMembership};

use super::types::{NodeId, NodeInfo};

/// Agent location in the cluster
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentLocation {
    /// Agent fingerprint
    pub fingerprint: String,

    /// Node hosting this agent
    pub node_id: NodeId,

    /// Last update timestamp
    pub updated_at: i64,

    /// Agent capabilities
    pub capabilities: Vec<String>,
}

/// Service entry in the registry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// Service type
    pub service_type: String,

    /// Service name
    pub name: String,

    /// Providing agent fingerprint
    pub provider: String,

    /// Node hosting the provider
    pub node_id: NodeId,

    /// Service properties
    pub properties: HashMap<String, String>,

    /// Registration timestamp
    pub registered_at: i64,
}

/// Request types for the state machine
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StateRequest {
    /// Register agent location
    RegisterAgent {
        fingerprint: String,
        node_id: NodeId,
        capabilities: Vec<String>,
    },

    /// Unregister agent
    UnregisterAgent {
        fingerprint: String,
    },

    /// Register service
    RegisterService {
        service_type: String,
        name: String,
        provider: String,
        properties: HashMap<String, String>,
    },

    /// Unregister service
    UnregisterService {
        service_type: String,
        provider: String,
    },

    /// Query agent location
    QueryAgent {
        fingerprint: String,
    },

    /// Query services by type
    QueryServices {
        service_type: String,
    },
}

/// Response types from the state machine
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StateResponse {
    /// Success with no data
    Ok,

    /// Agent location result
    Agent(Option<AgentLocation>),

    /// Service list result
    Services(Vec<ServiceEntry>),

    /// Error
    Error(String),
}

/// Cluster state managed by Raft
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClusterState {
    /// Agent directory: fingerprint -> location
    pub agents: HashMap<String, AgentLocation>,

    /// Service registry: service_type -> entries
    pub services: HashMap<String, Vec<ServiceEntry>>,

    /// Last applied log index
    pub last_applied_log: Option<LogId<NodeId>>,

    /// Current membership
    pub last_membership: StoredMembership<NodeId, NodeInfo>,
}

impl ClusterState {
    /// Apply a state request
    pub fn apply(&mut self, request: StateRequest, node_id: NodeId) -> StateResponse {
        let now = chrono::Utc::now().timestamp();

        match request {
            StateRequest::RegisterAgent { fingerprint, node_id, capabilities } => {
                self.agents.insert(fingerprint.clone(), AgentLocation {
                    fingerprint,
                    node_id,
                    updated_at: now,
                    capabilities,
                });
                StateResponse::Ok
            }

            StateRequest::UnregisterAgent { fingerprint } => {
                self.agents.remove(&fingerprint);
                StateResponse::Ok
            }

            StateRequest::RegisterService { service_type, name, provider, properties } => {
                let entry = ServiceEntry {
                    service_type: service_type.clone(),
                    name,
                    provider: provider.clone(),
                    node_id,
                    properties,
                    registered_at: now,
                };

                self.services
                    .entry(service_type)
                    .or_default()
                    .retain(|e| e.provider != provider);

                self.services
                    .entry(entry.service_type.clone())
                    .or_default()
                    .push(entry);

                StateResponse::Ok
            }

            StateRequest::UnregisterService { service_type, provider } => {
                if let Some(services) = self.services.get_mut(&service_type) {
                    services.retain(|e| e.provider != provider);
                }
                StateResponse::Ok
            }

            StateRequest::QueryAgent { fingerprint } => {
                StateResponse::Agent(self.agents.get(&fingerprint).cloned())
            }

            StateRequest::QueryServices { service_type } => {
                StateResponse::Services(
                    self.services.get(&service_type).cloned().unwrap_or_default()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_agent() {
        let mut state = ClusterState::default();
        let response = state.apply(
            StateRequest::RegisterAgent {
                fingerprint: "abc123".into(),
                node_id: 1,
                capabilities: vec!["messaging".into()],
            },
            1,
        );

        assert!(matches!(response, StateResponse::Ok));
        assert!(state.agents.contains_key("abc123"));
    }

    #[test]
    fn test_register_service() {
        let mut state = ClusterState::default();
        state.apply(
            StateRequest::RegisterService {
                service_type: "directory".into(),
                name: "agent-directory".into(),
                provider: "agent1".into(),
                properties: HashMap::new(),
            },
            1,
        );

        let response = state.apply(
            StateRequest::QueryServices {
                service_type: "directory".into(),
            },
            1,
        );

        if let StateResponse::Services(services) = response {
            assert_eq!(services.len(), 1);
            assert_eq!(services[0].name, "agent-directory");
        } else {
            panic!("Expected Services response");
        }
    }
}
