// security/auth.rs - Authentication and Session Management
//
//! Agent authentication and session management
//!
//! Provides:
//! - Multi-method authentication (certificates, tokens)
//! - Session creation and validation
//! - Lockout protection against brute force

use super::credentials::{AgentCredentials, CredentialError};
use crate::proto::AgentId;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;
use tracing::{debug, warn};

/// Authentication errors
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials: {0}")]
    InvalidCredentials(String),

    #[error("Credential error: {0}")]
    CredentialError(#[from] CredentialError),

    #[error("Account locked out until {0:?}")]
    AccountLocked(Instant),

    #[error("Session expired")]
    SessionExpired,

    #[error("Session not found")]
    SessionNotFound,

    #[error("Authentication required")]
    AuthRequired,

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for authentication operations
pub type AuthResult<T> = Result<T, AuthError>;

/// Session identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a new random session ID
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Create from string
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Authentication session
#[derive(Debug, Clone)]
pub struct Session {
    /// Session ID
    pub id: SessionId,

    /// Agent ID
    pub agent_id: AgentId,

    /// Creation time
    pub created_at: SystemTime,

    /// Last activity time
    pub last_activity: SystemTime,

    /// Expiration time
    pub expires_at: SystemTime,

    /// Is anonymous session
    pub anonymous: bool,

    /// Principal name (from credentials)
    pub principal: Option<String>,

    /// Associated roles (for RBAC)
    pub roles: Vec<String>,

    /// Session metadata
    pub metadata: HashMap<String, String>,
}

impl Session {
    /// Create a new authenticated session
    pub fn new(agent_id: AgentId, principal: Option<String>, timeout_secs: u64) -> Self {
        let now = SystemTime::now();
        Self {
            id: SessionId::new(),
            agent_id,
            created_at: now,
            last_activity: now,
            expires_at: now + Duration::from_secs(timeout_secs),
            anonymous: false,
            principal,
            roles: vec![],
            metadata: HashMap::new(),
        }
    }

    /// Create an anonymous session
    pub fn anonymous(agent_id: AgentId) -> Self {
        let now = SystemTime::now();
        Self {
            id: SessionId::new(),
            agent_id,
            created_at: now,
            last_activity: now,
            expires_at: now + Duration::from_secs(3600), // 1 hour default
            anonymous: true,
            principal: None,
            roles: vec!["anonymous".to_string()],
            metadata: HashMap::new(),
        }
    }

    /// Check if session is anonymous
    pub fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    /// Check if session is valid (not expired)
    pub fn is_valid(&self) -> bool {
        SystemTime::now() < self.expires_at
    }

    /// Refresh the session (update last activity)
    pub fn refresh(&mut self) {
        self.last_activity = SystemTime::now();
    }

    /// Extend session expiration
    pub fn extend(&mut self, additional_secs: u64) {
        self.expires_at = SystemTime::now() + Duration::from_secs(additional_secs);
    }

    /// Add a role to the session
    pub fn add_role(&mut self, role: String) {
        if !self.roles.contains(&role) {
            self.roles.push(role);
        }
    }

    /// Check if session has a specific role
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Set metadata
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }

    /// Get metadata
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}

/// Lockout tracking for an agent
#[derive(Debug)]
struct LockoutInfo {
    /// Number of failed attempts
    failed_attempts: u32,
    /// Lockout until this time
    locked_until: Option<Instant>,
    /// Last failed attempt
    last_failure: Instant,
}

impl LockoutInfo {
    fn new() -> Self {
        Self {
            failed_attempts: 0,
            locked_until: None,
            last_failure: Instant::now(),
        }
    }

    fn record_failure(&mut self, max_attempts: u32, lockout_duration: Duration) {
        self.failed_attempts += 1;
        self.last_failure = Instant::now();

        if self.failed_attempts >= max_attempts {
            self.locked_until = Some(Instant::now() + lockout_duration);
        }
    }

    fn is_locked(&self) -> bool {
        if let Some(locked_until) = self.locked_until {
            Instant::now() < locked_until
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.failed_attempts = 0;
        self.locked_until = None;
    }
}

/// Authenticator - handles credential verification
#[derive(Debug)]
pub struct Authenticator {
    /// Allow self-signed certificates
    allow_self_signed: bool,

    /// Maximum failed attempts before lockout
    max_failures: u32,

    /// Lockout duration
    lockout_duration: Duration,

    /// Session timeout
    session_timeout: Duration,

    /// Lockout tracking per agent
    lockouts: HashMap<String, LockoutInfo>,

    /// Valid API keys (in production, use secure storage)
    api_keys: HashMap<String, String>, // key -> agent principal

    /// Valid bearer tokens (in production, use secure storage)
    bearer_tokens: HashMap<String, String>, // token -> agent principal
}

impl Authenticator {
    /// Create a new authenticator
    pub fn new(allow_self_signed: bool, max_failures: u32, lockout_duration_secs: u64) -> Self {
        Self {
            allow_self_signed,
            max_failures,
            lockout_duration: Duration::from_secs(lockout_duration_secs),
            session_timeout: Duration::from_secs(3600),
            lockouts: HashMap::new(),
            api_keys: HashMap::new(),
            bearer_tokens: HashMap::new(),
        }
    }

    /// Set session timeout
    pub fn with_session_timeout(mut self, timeout_secs: u64) -> Self {
        self.session_timeout = Duration::from_secs(timeout_secs);
        self
    }

    /// Register an API key
    pub fn register_api_key(&mut self, key: String, principal: String) {
        self.api_keys.insert(key, principal);
    }

    /// Register a bearer token
    pub fn register_bearer_token(&mut self, token: String, principal: String) {
        self.bearer_tokens.insert(token, principal);
    }

    /// Revoke an API key
    pub fn revoke_api_key(&mut self, key: &str) -> bool {
        self.api_keys.remove(key).is_some()
    }

    /// Revoke a bearer token
    pub fn revoke_bearer_token(&mut self, token: &str) -> bool {
        self.bearer_tokens.remove(token).is_some()
    }

    /// Authenticate an agent
    pub fn authenticate(
        &mut self,
        agent_id: &AgentId,
        credentials: &AgentCredentials,
    ) -> AuthResult<Session> {
        let agent_name = &agent_id.name;

        // Check for lockout
        if let Some(lockout) = self.lockouts.get(agent_name) {
            if lockout.is_locked() {
                warn!("Agent '{}' is locked out", agent_name);
                return Err(AuthError::AccountLocked(lockout.locked_until.unwrap()));
            }
        }

        // Validate credentials
        match self.verify_credentials(credentials) {
            Ok(principal) => {
                // Clear any lockout on success
                if let Some(lockout) = self.lockouts.get_mut(agent_name) {
                    lockout.clear();
                }

                let session = Session::new(
                    agent_id.clone(),
                    principal,
                    self.session_timeout.as_secs(),
                );

                debug!(
                    "Authentication successful for agent '{}', session: {}",
                    agent_name, session.id
                );

                Ok(session)
            }
            Err(e) => {
                // Record failure
                let lockout = self
                    .lockouts
                    .entry(agent_name.clone())
                    .or_insert_with(LockoutInfo::new);
                lockout.record_failure(self.max_failures, self.lockout_duration);

                warn!(
                    "Authentication failed for agent '{}': {} (attempt {}/{})",
                    agent_name, e, lockout.failed_attempts, self.max_failures
                );

                Err(e)
            }
        }
    }

    /// Verify credentials and return principal
    fn verify_credentials(&self, credentials: &AgentCredentials) -> AuthResult<Option<String>> {
        match credentials {
            AgentCredentials::Anonymous => Ok(None),

            AgentCredentials::Certificate(cert) => {
                cert.validate(self.allow_self_signed)?;
                Ok(Some(cert.subject_cn.clone()))
            }

            AgentCredentials::Token(token) => {
                token.validate()?;

                // Check against registered tokens
                match token.token_type {
                    super::credentials::TokenType::Bearer => {
                        if let Some(principal) = self.bearer_tokens.get(&token.value) {
                            Ok(Some(principal.clone()))
                        } else {
                            Err(AuthError::InvalidCredentials(
                                "Unknown bearer token".to_string(),
                            ))
                        }
                    }
                    super::credentials::TokenType::ApiKey => {
                        if let Some(principal) = self.api_keys.get(&token.value) {
                            Ok(Some(principal.clone()))
                        } else {
                            Err(AuthError::InvalidCredentials("Unknown API key".to_string()))
                        }
                    }
                    super::credentials::TokenType::Jwt => {
                        // JWT validation would go here
                        // For now, check claims for subject
                        if let Some(sub) = token.claims.get("sub") {
                            Ok(Some(sub.clone()))
                        } else {
                            Err(AuthError::InvalidCredentials(
                                "JWT missing subject claim".to_string(),
                            ))
                        }
                    }
                }
            }

            AgentCredentials::CertificateWithToken { certificate, token } => {
                // Validate both
                certificate.validate(self.allow_self_signed)?;
                token.validate()?;
                Ok(Some(certificate.subject_cn.clone()))
            }
        }
    }

    /// Check if an agent is currently locked out
    pub fn is_locked_out(&self, agent_name: &str) -> bool {
        self.lockouts
            .get(agent_name)
            .map(|l| l.is_locked())
            .unwrap_or(false)
    }

    /// Clear lockout for an agent (admin function)
    pub fn clear_lockout(&mut self, agent_name: &str) -> bool {
        if let Some(lockout) = self.lockouts.get_mut(agent_name) {
            lockout.clear();
            true
        } else {
            false
        }
    }

    /// Get failed attempt count for an agent
    pub fn failed_attempts(&self, agent_name: &str) -> u32 {
        self.lockouts
            .get(agent_name)
            .map(|l| l.failed_attempts)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent_id(name: &str) -> AgentId {
        AgentId {
            name: name.to_string(),
            addresses: vec![],
            resolvers: vec![],
        }
    }

    #[test]
    fn test_session_creation() {
        let agent_id = test_agent_id("test-agent");
        let session = Session::new(agent_id, Some("principal".to_string()), 3600);

        assert!(!session.is_anonymous());
        assert!(session.is_valid());
        assert_eq!(session.principal, Some("principal".to_string()));
    }

    #[test]
    fn test_anonymous_session() {
        let agent_id = test_agent_id("anon-agent");
        let session = Session::anonymous(agent_id);

        assert!(session.is_anonymous());
        assert!(session.is_valid());
        assert!(session.has_role("anonymous"));
    }

    #[test]
    fn test_session_roles() {
        let agent_id = test_agent_id("role-agent");
        let mut session = Session::new(agent_id, None, 3600);

        session.add_role("admin".to_string());
        session.add_role("user".to_string());

        assert!(session.has_role("admin"));
        assert!(session.has_role("user"));
        assert!(!session.has_role("guest"));
    }

    #[test]
    fn test_authenticator_anonymous() {
        let mut auth = Authenticator::new(true, 5, 300);
        let agent_id = test_agent_id("anon");
        let creds = AgentCredentials::anonymous();

        let result = auth.authenticate(&agent_id, &creds);
        assert!(result.is_ok());

        let session = result.unwrap();
        assert!(session.principal.is_none());
    }

    #[test]
    fn test_authenticator_api_key() {
        let mut auth = Authenticator::new(true, 5, 300);
        auth.register_api_key("test-key-123".to_string(), "api-user".to_string());

        let agent_id = test_agent_id("api-agent");
        let creds = AgentCredentials::from_api_key("test-key-123".to_string());

        let result = auth.authenticate(&agent_id, &creds);
        assert!(result.is_ok());

        let session = result.unwrap();
        assert_eq!(session.principal, Some("api-user".to_string()));
    }

    #[test]
    fn test_authenticator_invalid_key() {
        let mut auth = Authenticator::new(true, 5, 300);
        let agent_id = test_agent_id("bad-agent");
        let creds = AgentCredentials::from_api_key("invalid-key".to_string());

        let result = auth.authenticate(&agent_id, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_lockout() {
        let mut auth = Authenticator::new(true, 3, 60);
        let agent_id = test_agent_id("lockout-agent");
        let bad_creds = AgentCredentials::from_api_key("wrong".to_string());

        // Fail 3 times
        for _ in 0..3 {
            let _ = auth.authenticate(&agent_id, &bad_creds);
        }

        // Should be locked out
        assert!(auth.is_locked_out("lockout-agent"));

        // Next attempt should fail with locked error
        let result = auth.authenticate(&agent_id, &bad_creds);
        assert!(matches!(result, Err(AuthError::AccountLocked(_))));

        // Clear lockout
        auth.clear_lockout("lockout-agent");
        assert!(!auth.is_locked_out("lockout-agent"));
    }
}
