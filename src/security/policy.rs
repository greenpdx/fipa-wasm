// security/policy.rs - RBAC Policy Engine
//
//! Role-Based Access Control (RBAC) Policy Management
//!
//! Provides:
//! - Role definitions with permissions
//! - Role bindings to agents
//! - Policy file loading (YAML/TOML)
//! - Permission resolution

use super::permissions::{Action, Permission, PermissionSet, Resource};
use std::collections::HashMap;
use std::fs;
use thiserror::Error;
use tracing::info;

/// Policy-related errors
#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("Failed to load policy file: {0}")]
    LoadError(String),

    #[error("Invalid policy format: {0}")]
    InvalidFormat(String),

    #[error("Role not found: {0}")]
    RoleNotFound(String),

    #[error("Circular role inheritance detected: {0}")]
    CircularInheritance(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// A role definition
#[derive(Debug, Clone)]
pub struct Role {
    /// Role name
    pub name: String,

    /// Description
    pub description: String,

    /// Permissions granted by this role
    pub permissions: PermissionSet,

    /// Roles this role inherits from
    pub inherits: Vec<String>,
}

impl Role {
    /// Create a new role
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            permissions: PermissionSet::new(),
            inherits: vec![],
        }
    }

    /// Set description
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    /// Add a permission
    pub fn with_permission(mut self, permission: Permission) -> Self {
        self.permissions.add(permission);
        self
    }

    /// Allow an action on a resource
    pub fn allow(mut self, resource: Resource, actions: Vec<Action>) -> Self {
        self.permissions.allow(resource, actions);
        self
    }

    /// Deny an action on a resource
    pub fn deny(mut self, resource: Resource, actions: Vec<Action>) -> Self {
        self.permissions.deny(resource, actions);
        self
    }

    /// Inherit from another role
    pub fn inherits_from(mut self, role_name: &str) -> Self {
        self.inherits.push(role_name.to_string());
        self
    }
}

/// Role binding - associates an agent with roles
#[derive(Debug, Clone)]
pub struct RoleBinding {
    /// Agent name or pattern
    pub agent_pattern: String,

    /// Assigned roles
    pub roles: Vec<String>,
}

impl RoleBinding {
    /// Create a new role binding
    pub fn new(agent_pattern: &str, roles: Vec<String>) -> Self {
        Self {
            agent_pattern: agent_pattern.to_string(),
            roles,
        }
    }

    /// Check if this binding matches an agent
    pub fn matches(&self, agent_name: &str) -> bool {
        if self.agent_pattern == "*" {
            return true;
        }

        if self.agent_pattern.contains('*') {
            let pattern = self.agent_pattern.replace("*", "");
            if self.agent_pattern.starts_with('*') && self.agent_pattern.ends_with('*') {
                agent_name.contains(&pattern)
            } else if self.agent_pattern.starts_with('*') {
                agent_name.ends_with(&pattern)
            } else if self.agent_pattern.ends_with('*') {
                agent_name.starts_with(&pattern)
            } else {
                self.agent_pattern == agent_name
            }
        } else {
            self.agent_pattern == agent_name
        }
    }
}

/// Security policy definition
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Policy name
    pub name: String,

    /// Policy version
    pub version: String,

    /// Roles defined in this policy
    pub roles: HashMap<String, Role>,

    /// Role bindings
    pub bindings: Vec<RoleBinding>,

    /// Default deny behavior
    pub default_deny: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            version: "1.0".to_string(),
            roles: HashMap::new(),
            bindings: vec![],
            default_deny: true,
        }
    }
}

impl SecurityPolicy {
    /// Create a new policy
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Add a role
    pub fn add_role(&mut self, role: Role) {
        self.roles.insert(role.name.clone(), role);
    }

    /// Add a role binding
    pub fn add_binding(&mut self, binding: RoleBinding) {
        self.bindings.push(binding);
    }

    /// Get a role by name
    pub fn get_role(&self, name: &str) -> Option<&Role> {
        self.roles.get(name)
    }
}

/// Represents a complete policy loaded from a YAML/TOML file
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// The security policy
    pub policy: SecurityPolicy,
}

impl Policy {
    /// Create an empty policy
    pub fn new() -> Self {
        Self {
            policy: SecurityPolicy::default(),
        }
    }

    /// Load policy from YAML content
    pub fn from_yaml(content: &str) -> Result<Self, PolicyError> {
        // Simple YAML parser for policy files
        // Format:
        // policy:
        //   name: my-policy
        //   version: 1.0
        //   default_deny: true
        //
        // roles:
        //   admin:
        //     description: Administrator role
        //     permissions:
        //       - resource: "*/*"
        //         actions: [admin]
        //   user:
        //     description: Regular user
        //     permissions:
        //       - resource: "agent/*"
        //         actions: [read]
        //
        // bindings:
        //   - agent: "admin-*"
        //     roles: [admin]
        //   - agent: "*"
        //     roles: [user]

        let mut policy = SecurityPolicy::default();
        let mut current_section = "";
        let mut current_role: Option<Role> = None;
        let mut current_binding: Option<(String, Vec<String>)> = None;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Section headers
            if trimmed == "policy:" {
                current_section = "policy";
                continue;
            } else if trimmed == "roles:" {
                // Save any pending role
                if let Some(role) = current_role.take() {
                    policy.add_role(role);
                }
                current_section = "roles";
                continue;
            } else if trimmed == "bindings:" {
                // Save any pending role
                if let Some(role) = current_role.take() {
                    policy.add_role(role);
                }
                current_section = "bindings";
                continue;
            }

            match current_section {
                "policy" => {
                    if let Some((key, value)) = parse_key_value(trimmed) {
                        match key.as_str() {
                            "name" => policy.name = value,
                            "version" => policy.version = value,
                            "default_deny" => {
                                policy.default_deny = value.to_lowercase() == "true"
                            }
                            _ => {}
                        }
                    }
                }
                "roles" => {
                    // Check if this is a new role definition
                    if !trimmed.starts_with('-') && !trimmed.starts_with(' ') && trimmed.ends_with(':') {
                        // Save previous role
                        if let Some(role) = current_role.take() {
                            policy.add_role(role);
                        }
                        let role_name = trimmed.trim_end_matches(':');
                        current_role = Some(Role::new(role_name));
                    } else if let Some(ref mut role) = current_role {
                        if let Some((key, value)) = parse_key_value(trimmed) {
                            match key.as_str() {
                                "description" => role.description = value,
                                "inherits" => {
                                    role.inherits = parse_list(&value);
                                }
                                _ => {}
                            }
                        } else if trimmed.starts_with("- resource:") {
                            // Parse permission
                            let resource_str = trimmed.trim_start_matches("- resource:").trim();
                            let resource_str = resource_str.trim_matches('"');
                            if let Ok(resource) = Resource::parse(resource_str) {
                                // Next line should have actions
                                role.permissions.allow(resource, vec![Action::Read]);
                            }
                        }
                    }
                }
                "bindings" => {
                    if trimmed.starts_with("- agent:") {
                        // Save previous binding
                        if let Some((pattern, roles)) = current_binding.take() {
                            policy.add_binding(RoleBinding::new(&pattern, roles));
                        }
                        let agent_pattern = trimmed.trim_start_matches("- agent:").trim();
                        let agent_pattern = agent_pattern.trim_matches('"');
                        current_binding = Some((agent_pattern.to_string(), vec![]));
                    } else if let Some((key, value)) = parse_key_value(trimmed) {
                        if key == "roles" {
                            if let Some((_, ref mut roles)) = current_binding {
                                *roles = parse_list(&value);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Save any pending items
        if let Some(role) = current_role {
            policy.add_role(role);
        }
        if let Some((pattern, roles)) = current_binding {
            policy.add_binding(RoleBinding::new(&pattern, roles));
        }

        Ok(Self { policy })
    }

    /// Load policy from file
    pub fn from_file(path: &str) -> Result<Self, PolicyError> {
        let content = fs::read_to_string(path)?;

        if path.ends_with(".yaml") || path.ends_with(".yml") {
            Self::from_yaml(&content)
        } else if path.ends_with(".toml") {
            // TOML support could be added here
            Err(PolicyError::InvalidFormat(
                "TOML support not yet implemented".to_string(),
            ))
        } else {
            // Try YAML by default
            Self::from_yaml(&content)
        }
    }
}

/// Helper to parse "key: value" lines
fn parse_key_value(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() == 2 {
        let key = parts[0].trim().to_string();
        let value = parts[1].trim().trim_matches('"').to_string();
        Some((key, value))
    } else {
        None
    }
}

/// Helper to parse "[a, b, c]" lists
fn parse_list(value: &str) -> Vec<String> {
    let trimmed = value.trim_matches(|c| c == '[' || c == ']');
    trimmed
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Policy engine - evaluates policies
#[derive(Debug, Default)]
pub struct PolicyEngine {
    /// Active policy
    policy: Option<SecurityPolicy>,

    /// Cached role permissions (resolved with inheritance)
    role_cache: HashMap<String, PermissionSet>,

    /// Cached agent permissions
    agent_cache: HashMap<String, PermissionSet>,
}

impl PolicyEngine {
    /// Create a new policy engine
    pub fn new() -> Self {
        Self {
            policy: None,
            role_cache: HashMap::new(),
            agent_cache: HashMap::new(),
        }
    }

    /// Load policy from file
    pub fn load_from_file(&mut self, path: &str) -> Result<(), PolicyError> {
        let policy = Policy::from_file(path)?;
        self.set_policy(policy.policy);
        info!("Loaded security policy from {}", path);
        Ok(())
    }

    /// Set the active policy
    pub fn set_policy(&mut self, policy: SecurityPolicy) {
        // Clear caches
        self.role_cache.clear();
        self.agent_cache.clear();

        // Pre-resolve role permissions
        for role_name in policy.roles.keys() {
            if let Ok(perms) = self.resolve_role_permissions(&policy, role_name, &mut vec![]) {
                self.role_cache.insert(role_name.clone(), perms);
            }
        }

        self.policy = Some(policy);
    }

    /// Resolve permissions for a role, handling inheritance
    fn resolve_role_permissions(
        &self,
        policy: &SecurityPolicy,
        role_name: &str,
        visited: &mut Vec<String>,
    ) -> Result<PermissionSet, PolicyError> {
        // Check for circular inheritance
        if visited.contains(&role_name.to_string()) {
            return Err(PolicyError::CircularInheritance(role_name.to_string()));
        }
        visited.push(role_name.to_string());

        let role = policy
            .get_role(role_name)
            .ok_or_else(|| PolicyError::RoleNotFound(role_name.to_string()))?;

        let mut perms = role.permissions.clone();

        // Resolve inherited permissions
        for inherited_name in &role.inherits {
            let inherited_perms = self.resolve_role_permissions(policy, inherited_name, visited)?;
            perms.merge(&inherited_perms);
        }

        Ok(perms)
    }

    /// Get permissions for an agent
    pub fn get_agent_permissions(&mut self, agent_name: &str) -> PermissionSet {
        // Check cache first
        if let Some(perms) = self.agent_cache.get(agent_name) {
            return perms.clone();
        }

        let mut perms = PermissionSet::new();

        if let Some(ref policy) = self.policy {
            // Find matching bindings
            for binding in &policy.bindings {
                if binding.matches(agent_name) {
                    // Add permissions from each role
                    for role_name in &binding.roles {
                        if let Some(role_perms) = self.role_cache.get(role_name) {
                            perms.merge(role_perms);
                        }
                    }
                }
            }
        }

        // Cache the result
        self.agent_cache.insert(agent_name.to_string(), perms.clone());
        perms
    }

    /// Check if an agent has permission
    pub fn check_permission(&mut self, agent_name: &str, resource: &str, action: &str) -> bool {
        let perms = self.get_agent_permissions(agent_name);
        let resource = match Resource::parse(resource) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let action = Action::from_str(action);

        perms.check(&resource, &action)
    }

    /// Add a built-in role
    pub fn add_builtin_role(&mut self, role: Role) {
        let perms = role.permissions.clone();
        self.role_cache.insert(role.name.clone(), perms);

        if let Some(ref mut policy) = self.policy {
            policy.add_role(role);
        } else {
            let mut policy = SecurityPolicy::default();
            policy.add_role(role);
            self.policy = Some(policy);
        }
    }

    /// Add a role binding programmatically
    pub fn add_binding(&mut self, binding: RoleBinding) {
        // Clear agent cache since bindings changed
        self.agent_cache.clear();

        if let Some(ref mut policy) = self.policy {
            policy.add_binding(binding);
        } else {
            let mut policy = SecurityPolicy::default();
            policy.add_binding(binding);
            self.policy = Some(policy);
        }
    }

    /// Create default policy with common roles
    pub fn with_defaults(mut self) -> Self {
        // Admin role
        let admin = Role::new("admin")
            .with_description("Full administrative access")
            .allow(Resource::all("*"), vec![Action::Admin]);
        self.add_builtin_role(admin);

        // Platform role (for AMS/DF)
        let platform = Role::new("platform")
            .with_description("Platform agent access")
            .allow(Resource::all("agent"), vec![Action::Read, Action::Create, Action::Delete])
            .allow(Resource::all("service"), vec![Action::Read, Action::Create, Action::Update, Action::Delete])
            .allow(Resource::all("message"), vec![Action::Read, Action::Execute]);
        self.add_builtin_role(platform);

        // User role
        let user = Role::new("user")
            .with_description("Standard user access")
            .allow(Resource::all("agent"), vec![Action::Read])
            .allow(Resource::all("service"), vec![Action::Read])
            .allow(Resource::new("platform", "df"), vec![Action::Read, Action::Execute]);
        self.add_builtin_role(user);

        // Anonymous role
        let anonymous = Role::new("anonymous")
            .with_description("Minimal anonymous access")
            .allow(Resource::new("platform", "df"), vec![Action::Read]);
        self.add_builtin_role(anonymous);

        // Default bindings
        self.add_binding(RoleBinding::new("ams", vec!["platform".to_string(), "admin".to_string()]));
        self.add_binding(RoleBinding::new("df", vec!["platform".to_string()]));
        self.add_binding(RoleBinding::new("*", vec!["user".to_string()]));

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_creation() {
        let role = Role::new("test-role")
            .with_description("A test role")
            .allow(Resource::new("agent", "*"), vec![Action::Read]);

        assert_eq!(role.name, "test-role");
        assert!(!role.description.is_empty());
    }

    #[test]
    fn test_role_binding_matching() {
        let binding = RoleBinding::new("test-*", vec!["user".to_string()]);

        assert!(binding.matches("test-agent-1"));
        assert!(binding.matches("test-"));
        assert!(!binding.matches("prod-agent"));
    }

    #[test]
    fn test_wildcard_binding() {
        let binding = RoleBinding::new("*", vec!["default".to_string()]);

        assert!(binding.matches("any-agent"));
        assert!(binding.matches(""));
    }

    #[test]
    fn test_policy_engine_defaults() {
        let mut engine = PolicyEngine::new().with_defaults();

        // AMS should have admin access
        assert!(engine.check_permission("ams", "agent/any", "admin"));

        // DF should have platform access
        assert!(engine.check_permission("df", "service/test", "create"));

        // Regular agents should have user access
        assert!(engine.check_permission("my-agent", "agent/other", "read"));
        assert!(!engine.check_permission("my-agent", "agent/other", "delete"));
    }

    #[test]
    fn test_policy_from_yaml() {
        let yaml = r#"
policy:
  name: test-policy
  version: 1.0
  default_deny: true

roles:
  tester:
    description: Test role

bindings:
  - agent: "test-*"
    roles: [tester]
"#;

        let policy = Policy::from_yaml(yaml).unwrap();
        assert_eq!(policy.policy.name, "test-policy");
        assert!(policy.policy.roles.contains_key("tester"));
        assert!(!policy.policy.bindings.is_empty());
    }

    #[test]
    fn test_permission_caching() {
        let mut engine = PolicyEngine::new().with_defaults();

        // First call builds cache
        let perms1 = engine.get_agent_permissions("test-agent");

        // Second call should use cache
        let perms2 = engine.get_agent_permissions("test-agent");

        assert_eq!(perms1.permissions().len(), perms2.permissions().len());
    }
}
