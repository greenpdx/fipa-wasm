// security/credentials.rs - Agent Credentials and Identity
//
//! Agent identity and credential management
//!
//! Supports multiple credential types:
//! - X.509 certificates
//! - Bearer tokens (JWT-like)
//! - Anonymous credentials

use std::time::{Duration, SystemTime};
use thiserror::Error;

/// Credential-related errors
#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Invalid certificate format")]
    InvalidCertificate,

    #[error("Certificate expired")]
    CertificateExpired,

    #[error("Invalid token format")]
    InvalidToken,

    #[error("Token expired")]
    TokenExpired,

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Unsupported credential type")]
    UnsupportedType,
}

/// Token types supported by the system
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    /// Bearer token (opaque string)
    Bearer,
    /// JWT-style token
    Jwt,
    /// API key
    ApiKey,
}

impl TokenType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenType::Bearer => "bearer",
            TokenType::Jwt => "jwt",
            TokenType::ApiKey => "api_key",
        }
    }
}

/// Token credential
#[derive(Debug, Clone)]
pub struct Token {
    /// Token type
    pub token_type: TokenType,

    /// Token value
    pub value: String,

    /// Expiration time
    pub expires_at: Option<SystemTime>,

    /// Associated claims/metadata
    pub claims: std::collections::HashMap<String, String>,
}

impl Token {
    /// Create a new bearer token
    pub fn bearer(value: String) -> Self {
        Self {
            token_type: TokenType::Bearer,
            value,
            expires_at: None,
            claims: std::collections::HashMap::new(),
        }
    }

    /// Create a new API key token
    pub fn api_key(value: String) -> Self {
        Self {
            token_type: TokenType::ApiKey,
            value,
            expires_at: None,
            claims: std::collections::HashMap::new(),
        }
    }

    /// Set expiration time
    pub fn with_expiry(mut self, expires_at: SystemTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Set expiration from duration
    pub fn expires_in(mut self, duration: Duration) -> Self {
        self.expires_at = Some(SystemTime::now() + duration);
        self
    }

    /// Add a claim
    pub fn with_claim(mut self, key: String, value: String) -> Self {
        self.claims.insert(key, value);
        self
    }

    /// Check if token is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            SystemTime::now() > expires_at
        } else {
            false
        }
    }

    /// Validate the token
    pub fn validate(&self) -> Result<(), CredentialError> {
        if self.value.is_empty() {
            return Err(CredentialError::InvalidToken);
        }

        if self.is_expired() {
            return Err(CredentialError::TokenExpired);
        }

        Ok(())
    }
}

/// X.509 Certificate representation
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Raw certificate bytes (DER or PEM encoded)
    pub raw: Vec<u8>,

    /// Subject common name
    pub subject_cn: String,

    /// Issuer common name
    pub issuer_cn: String,

    /// Serial number
    pub serial: String,

    /// Not valid before
    pub not_before: SystemTime,

    /// Not valid after
    pub not_after: SystemTime,

    /// Public key bytes
    pub public_key: Vec<u8>,

    /// Is self-signed
    pub self_signed: bool,
}

impl Certificate {
    /// Create a new certificate from raw bytes
    /// In a production implementation, this would parse X.509
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self, CredentialError> {
        if raw.is_empty() {
            return Err(CredentialError::InvalidCertificate);
        }

        // Simplified certificate parsing - in production, use x509-parser or similar
        // For now, we'll create a mock certificate for testing
        Ok(Self {
            raw: raw.clone(),
            subject_cn: "unknown".to_string(),
            issuer_cn: "unknown".to_string(),
            serial: hex::encode(&raw[..std::cmp::min(16, raw.len())]),
            not_before: SystemTime::UNIX_EPOCH,
            not_after: SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60),
            public_key: raw,
            self_signed: true,
        })
    }

    /// Create a self-signed certificate for testing
    pub fn self_signed_test(subject: &str) -> Self {
        let now = SystemTime::now();
        Self {
            raw: subject.as_bytes().to_vec(),
            subject_cn: subject.to_string(),
            issuer_cn: subject.to_string(),
            serial: format!("{:016x}", rand::random::<u64>()),
            not_before: now,
            not_after: now + Duration::from_secs(365 * 24 * 60 * 60),
            public_key: vec![0u8; 32], // Placeholder
            self_signed: true,
        }
    }

    /// Check if certificate is expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now();
        now < self.not_before || now > self.not_after
    }

    /// Validate the certificate
    pub fn validate(&self, allow_self_signed: bool) -> Result<(), CredentialError> {
        if self.raw.is_empty() {
            return Err(CredentialError::InvalidCertificate);
        }

        if self.is_expired() {
            return Err(CredentialError::CertificateExpired);
        }

        if self.self_signed && !allow_self_signed {
            return Err(CredentialError::InvalidCertificate);
        }

        Ok(())
    }
}

/// Agent credentials - the main credential type
#[derive(Debug, Clone)]
pub enum AgentCredentials {
    /// Anonymous (no credentials)
    Anonymous,

    /// Certificate-based authentication
    Certificate(Certificate),

    /// Token-based authentication
    Token(Token),

    /// Combined certificate + token
    CertificateWithToken {
        certificate: Certificate,
        token: Token,
    },
}

impl AgentCredentials {
    /// Create anonymous credentials
    pub fn anonymous() -> Self {
        AgentCredentials::Anonymous
    }

    /// Create credentials from a certificate
    pub fn from_certificate(cert_bytes: Vec<u8>) -> Result<Self, CredentialError> {
        let cert = Certificate::from_bytes(cert_bytes)?;
        Ok(AgentCredentials::Certificate(cert))
    }

    /// Create credentials from a bearer token
    pub fn from_bearer_token(token: String) -> Self {
        AgentCredentials::Token(Token::bearer(token))
    }

    /// Create credentials from an API key
    pub fn from_api_key(key: String) -> Self {
        AgentCredentials::Token(Token::api_key(key))
    }

    /// Check if credentials are anonymous
    pub fn is_anonymous(&self) -> bool {
        matches!(self, AgentCredentials::Anonymous)
    }

    /// Get the principal name from credentials
    pub fn principal(&self) -> Option<String> {
        match self {
            AgentCredentials::Anonymous => None,
            AgentCredentials::Certificate(cert) => Some(cert.subject_cn.clone()),
            AgentCredentials::Token(token) => token.claims.get("sub").cloned(),
            AgentCredentials::CertificateWithToken { certificate, .. } => {
                Some(certificate.subject_cn.clone())
            }
        }
    }

    /// Validate the credentials
    pub fn validate(&self, allow_self_signed: bool) -> Result<(), CredentialError> {
        match self {
            AgentCredentials::Anonymous => Ok(()),
            AgentCredentials::Certificate(cert) => cert.validate(allow_self_signed),
            AgentCredentials::Token(token) => token.validate(),
            AgentCredentials::CertificateWithToken { certificate, token } => {
                certificate.validate(allow_self_signed)?;
                token.validate()?;
                Ok(())
            }
        }
    }

    /// Get credential type as string
    pub fn credential_type(&self) -> &'static str {
        match self {
            AgentCredentials::Anonymous => "anonymous",
            AgentCredentials::Certificate(_) => "certificate",
            AgentCredentials::Token(_) => "token",
            AgentCredentials::CertificateWithToken { .. } => "certificate_with_token",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anonymous_credentials() {
        let creds = AgentCredentials::anonymous();
        assert!(creds.is_anonymous());
        assert!(creds.principal().is_none());
        assert!(creds.validate(true).is_ok());
    }

    #[test]
    fn test_bearer_token() {
        let token = Token::bearer("test-token-123".to_string());
        assert_eq!(token.token_type, TokenType::Bearer);
        assert!(!token.is_expired());
        assert!(token.validate().is_ok());
    }

    #[test]
    fn test_expired_token() {
        let token = Token::bearer("expired".to_string())
            .with_expiry(SystemTime::UNIX_EPOCH);
        assert!(token.is_expired());
        assert!(matches!(token.validate(), Err(CredentialError::TokenExpired)));
    }

    #[test]
    fn test_self_signed_certificate() {
        let cert = Certificate::self_signed_test("test-agent");
        assert_eq!(cert.subject_cn, "test-agent");
        assert!(cert.self_signed);
        assert!(!cert.is_expired());

        // Should validate with allow_self_signed = true
        assert!(cert.validate(true).is_ok());

        // Should fail with allow_self_signed = false
        assert!(cert.validate(false).is_err());
    }

    #[test]
    fn test_credentials_from_token() {
        let creds = AgentCredentials::from_bearer_token("my-token".to_string());
        assert!(!creds.is_anonymous());
        assert_eq!(creds.credential_type(), "token");
    }
}
