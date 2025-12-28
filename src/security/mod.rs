// security/mod.rs - FIPA Security Framework
//
//! Security Framework for FIPA Agent Platforms
//!
//! This module provides comprehensive security features:
//!
//! - **Authentication**: Agent identity verification using certificates and tokens
//! - **Authorization**: Permission-based access control for resources and actions
//! - **Message Security**: Signing and optional encryption of ACL messages
//! - **Policy Management**: YAML/TOML-based security policies
//!
//! # Architecture
//!
//! ```text
//! +------------------+     +------------------+     +------------------+
//! |  Authentication  |---->|  Authorization   |---->|  Policy Engine   |
//! |  (Credentials)   |     |  (Permissions)   |     |  (RBAC Rules)    |
//! +------------------+     +------------------+     +------------------+
//!         |                        |                        |
//!         v                        v                        v
//! +------------------+     +------------------+     +------------------+
//! |  Identity Store  |     |  Permission DB   |     |  Policy Files    |
//! +------------------+     +------------------+     +------------------+
//! ```
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::security::{SecurityManager, SecurityConfig};
//!
//! let config = SecurityConfig::default();
//! let manager = SecurityManager::new(config);
//!
//! // Authenticate an agent
//! let credentials = AgentCredentials::from_certificate(cert_bytes)?;
//! let session = manager.authenticate(&credentials)?;
//!
//! // Check permissions
//! if manager.authorize(&session, "df", "register")? {
//!     // Proceed with registration
//! }
//! ```

pub mod auth;
pub mod credentials;
pub mod permissions;
pub mod policy;

pub use auth::{AuthError, AuthResult, Authenticator, Session, SessionId};
pub use credentials::{AgentCredentials, Certificate, Token, TokenType};
pub use permissions::{
    Action, Permission, PermissionCheck, PermissionError, PermissionSet, Resource,
};
pub use policy::{Policy, PolicyEngine, PolicyError, Role, RoleBinding, SecurityPolicy};

use crate::proto::AgentId;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Security configuration
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Enable authentication
    pub auth_enabled: bool,

    /// Enable authorization
    pub authz_enabled: bool,

    /// Enable message signing
    pub message_signing: bool,

    /// Enable message encryption
    pub message_encryption: bool,

    /// Session timeout in seconds
    pub session_timeout_secs: u64,

    /// Maximum failed auth attempts before lockout
    pub max_auth_failures: u32,

    /// Lockout duration in seconds
    pub lockout_duration_secs: u64,

    /// Policy file path (optional)
    pub policy_file: Option<String>,

    /// Trusted certificate authorities
    pub trusted_cas: Vec<Vec<u8>>,

    /// Allow self-signed certificates
    pub allow_self_signed: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            auth_enabled: true,
            authz_enabled: true,
            message_signing: true,
            message_encryption: false,
            session_timeout_secs: 3600, // 1 hour
            max_auth_failures: 5,
            lockout_duration_secs: 300, // 5 minutes
            policy_file: None,
            trusted_cas: vec![],
            allow_self_signed: true, // For development
        }
    }
}

/// Security manager - main entry point for security operations
pub struct SecurityManager {
    /// Configuration
    config: SecurityConfig,

    /// Authenticator
    authenticator: Arc<RwLock<Authenticator>>,

    /// Policy engine
    policy_engine: Arc<RwLock<PolicyEngine>>,

    /// Active sessions
    sessions: Arc<RwLock<std::collections::HashMap<SessionId, Session>>>,
}

impl SecurityManager {
    /// Create a new security manager
    pub fn new(config: SecurityConfig) -> Self {
        let authenticator = Authenticator::new(
            config.allow_self_signed,
            config.max_auth_failures,
            config.lockout_duration_secs,
        );

        let policy_engine = PolicyEngine::new();

        Self {
            config,
            authenticator: Arc::new(RwLock::new(authenticator)),
            policy_engine: Arc::new(RwLock::new(policy_engine)),
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Load policy from file
    pub async fn load_policy(&self, path: &str) -> Result<(), PolicyError> {
        let mut engine = self.policy_engine.write().await;
        engine.load_from_file(path)?;
        info!("Loaded security policy from {}", path);
        Ok(())
    }

    /// Authenticate an agent with credentials
    pub async fn authenticate(
        &self,
        agent_id: &AgentId,
        credentials: &AgentCredentials,
    ) -> AuthResult<Session> {
        if !self.config.auth_enabled {
            // Auth disabled - create anonymous session
            return Ok(Session::anonymous(agent_id.clone()));
        }

        let mut auth = self.authenticator.write().await;
        let session = auth.authenticate(agent_id, credentials)?;

        // Store session
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session.clone());

        debug!("Agent '{}' authenticated, session: {}", agent_id.name, session.id);
        Ok(session)
    }

    /// Validate a session
    pub async fn validate_session(&self, session_id: &SessionId) -> Option<Session> {
        let sessions = self.sessions.read().await;

        if let Some(session) = sessions.get(session_id) {
            if session.is_valid() {
                return Some(session.clone());
            }
        }

        None
    }

    /// Invalidate a session (logout)
    pub async fn invalidate_session(&self, session_id: &SessionId) -> bool {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id).is_some()
    }

    /// Check if an action is authorized
    pub async fn authorize(
        &self,
        session: &Session,
        resource: &str,
        action: &str,
    ) -> Result<bool, PermissionError> {
        if !self.config.authz_enabled {
            // Authorization disabled - allow all
            return Ok(true);
        }

        // Check session validity
        if !session.is_valid() {
            return Err(PermissionError::SessionExpired);
        }

        // Check against policy engine
        let mut engine = self.policy_engine.write().await;
        let allowed = engine.check_permission(&session.agent_id.name, resource, action);

        if !allowed {
            warn!(
                "Authorization denied: agent='{}' resource='{}' action='{}'",
                session.agent_id.name, resource, action
            );
        }

        Ok(allowed)
    }

    /// Get session for an agent
    pub async fn get_session(&self, agent_id: &AgentId) -> Option<Session> {
        let sessions = self.sessions.read().await;

        sessions
            .values()
            .find(|s| s.agent_id.name == agent_id.name && s.is_valid())
            .cloned()
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired_sessions(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let before = sessions.len();

        sessions.retain(|_, session| session.is_valid());

        let removed = before - sessions.len();
        if removed > 0 {
            debug!("Cleaned up {} expired sessions", removed);
        }

        removed
    }

    /// Check if authentication is required
    pub fn auth_required(&self) -> bool {
        self.config.auth_enabled
    }

    /// Check if authorization is required
    pub fn authz_required(&self) -> bool {
        self.config.authz_enabled
    }

    /// Check if message signing is required
    pub fn signing_required(&self) -> bool {
        self.config.message_signing
    }

    /// Get the policy engine
    pub fn policy_engine(&self) -> Arc<RwLock<PolicyEngine>> {
        self.policy_engine.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_config_default() {
        let config = SecurityConfig::default();
        assert!(config.auth_enabled);
        assert!(config.authz_enabled);
        assert!(config.message_signing);
        assert!(!config.message_encryption);
        assert_eq!(config.session_timeout_secs, 3600);
    }

    #[tokio::test]
    async fn test_security_manager_creation() {
        let config = SecurityConfig::default();
        let manager = SecurityManager::new(config);

        assert!(manager.auth_required());
        assert!(manager.authz_required());
    }

    #[tokio::test]
    async fn test_disabled_auth() {
        let config = SecurityConfig {
            auth_enabled: false,
            ..Default::default()
        };
        let manager = SecurityManager::new(config);

        let agent_id = AgentId {
            name: "test-agent".to_string(),
            addresses: vec![],
            resolvers: vec![],
        };

        let credentials = AgentCredentials::anonymous();
        let session = manager.authenticate(&agent_id, &credentials).await.unwrap();

        assert!(session.is_anonymous());
    }
}
