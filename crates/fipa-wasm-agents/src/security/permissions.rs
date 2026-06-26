// security/permissions.rs - Permission-based Access Control
//
//! Permission system for resource access control
//!
//! Provides:
//! - Resource and action definitions
//! - Permission sets and checking
//! - Hierarchical resource matching

use std::collections::HashSet;
use thiserror::Error;

/// Permission-related errors
#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("Permission denied: {resource}/{action}")]
    Denied { resource: String, action: String },

    #[error("Session expired")]
    SessionExpired,

    #[error("Invalid resource pattern: {0}")]
    InvalidPattern(String),

    #[error("Unknown resource: {0}")]
    UnknownResource(String),
}

/// Standard actions on resources
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Create a new resource
    Create,
    /// Read/query a resource
    Read,
    /// Update an existing resource
    Update,
    /// Delete a resource
    Delete,
    /// Execute/invoke an action
    Execute,
    /// Subscribe to changes
    Subscribe,
    /// Admin-level access
    Admin,
    /// Custom action
    Custom(String),
}

impl Action {
    pub fn as_str(&self) -> &str {
        match self {
            Action::Create => "create",
            Action::Read => "read",
            Action::Update => "update",
            Action::Delete => "delete",
            Action::Execute => "execute",
            Action::Subscribe => "subscribe",
            Action::Admin => "admin",
            Action::Custom(s) => s,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "create" => Action::Create,
            "read" => Action::Read,
            "update" => Action::Update,
            "delete" => Action::Delete,
            "execute" => Action::Execute,
            "subscribe" => Action::Subscribe,
            "admin" => Action::Admin,
            _ => Action::Custom(s.to_string()),
        }
    }

    /// Check if this action implies another (e.g., admin implies all)
    pub fn implies(&self, other: &Action) -> bool {
        if self == other {
            return true;
        }

        match self {
            Action::Admin => true, // Admin implies everything
            _ => false,
        }
    }
}

/// Resource identifier with optional wildcards
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Resource {
    /// Resource type (e.g., "agent", "service", "message")
    pub resource_type: String,

    /// Resource name or pattern (supports * wildcard)
    pub name: String,
}

impl Resource {
    /// Create a new resource
    pub fn new(resource_type: &str, name: &str) -> Self {
        Self {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
        }
    }

    /// Create a wildcard resource (matches all of type)
    pub fn all(resource_type: &str) -> Self {
        Self {
            resource_type: resource_type.to_string(),
            name: "*".to_string(),
        }
    }

    /// Parse from string format "type/name"
    pub fn parse(s: &str) -> Result<Self, PermissionError> {
        let parts: Vec<&str> = s.splitn(2, '/').collect();
        if parts.len() == 2 {
            Ok(Self::new(parts[0], parts[1]))
        } else if parts.len() == 1 {
            Ok(Self::all(parts[0]))
        } else {
            Err(PermissionError::InvalidPattern(s.to_string()))
        }
    }

    /// Convert to string format
    pub fn to_string(&self) -> String {
        format!("{}/{}", self.resource_type, self.name)
    }

    /// Check if this resource pattern matches another resource
    pub fn matches(&self, other: &Resource) -> bool {
        // Type must match exactly
        if self.resource_type != other.resource_type && self.resource_type != "*" {
            return false;
        }

        // Check name pattern
        if self.name == "*" {
            return true;
        }

        if self.name.contains('*') {
            // Simple wildcard matching
            let pattern = self.name.replace("*", "");
            if self.name.starts_with('*') && self.name.ends_with('*') {
                other.name.contains(&pattern)
            } else if self.name.starts_with('*') {
                other.name.ends_with(&pattern)
            } else if self.name.ends_with('*') {
                other.name.starts_with(&pattern)
            } else {
                self.name == other.name
            }
        } else {
            self.name == other.name
        }
    }
}

/// A single permission (resource + allowed actions)
#[derive(Debug, Clone)]
pub struct Permission {
    /// Resource this permission applies to
    pub resource: Resource,

    /// Allowed actions
    pub actions: HashSet<Action>,

    /// Whether this is a deny permission (explicit denial)
    pub deny: bool,
}

impl Permission {
    /// Create a new allow permission
    pub fn allow(resource: Resource, actions: Vec<Action>) -> Self {
        Self {
            resource,
            actions: actions.into_iter().collect(),
            deny: false,
        }
    }

    /// Create a deny permission
    pub fn deny(resource: Resource, actions: Vec<Action>) -> Self {
        Self {
            resource,
            actions: actions.into_iter().collect(),
            deny: true,
        }
    }

    /// Create an allow-all permission for a resource
    pub fn allow_all(resource: Resource) -> Self {
        Self {
            resource,
            actions: [
                Action::Create,
                Action::Read,
                Action::Update,
                Action::Delete,
                Action::Execute,
                Action::Subscribe,
            ]
            .into_iter()
            .collect(),
            deny: false,
        }
    }

    /// Check if this permission allows a specific action
    pub fn allows(&self, action: &Action) -> bool {
        if self.deny {
            return false;
        }

        self.actions.iter().any(|a| a.implies(action))
    }

    /// Check if this permission denies a specific action
    pub fn denies(&self, action: &Action) -> bool {
        if !self.deny {
            return false;
        }

        self.actions.iter().any(|a| a == action || matches!(a, Action::Admin))
    }
}

/// A set of permissions for an entity
#[derive(Debug, Clone, Default)]
pub struct PermissionSet {
    /// List of permissions
    permissions: Vec<Permission>,
}

impl PermissionSet {
    /// Create an empty permission set
    pub fn new() -> Self {
        Self {
            permissions: vec![],
        }
    }

    /// Create a permission set with admin access to everything
    pub fn admin() -> Self {
        let mut set = Self::new();
        set.add(Permission::allow(
            Resource::all("*"),
            vec![Action::Admin],
        ));
        set
    }

    /// Add a permission
    pub fn add(&mut self, permission: Permission) {
        self.permissions.push(permission);
    }

    /// Add an allow permission
    pub fn allow(&mut self, resource: Resource, actions: Vec<Action>) {
        self.add(Permission::allow(resource, actions));
    }

    /// Add a deny permission
    pub fn deny(&mut self, resource: Resource, actions: Vec<Action>) {
        self.add(Permission::deny(resource, actions));
    }

    /// Check if an action is allowed on a resource
    pub fn check(&self, resource: &Resource, action: &Action) -> bool {
        // First check for explicit denials
        for perm in &self.permissions {
            if perm.deny && perm.resource.matches(resource) && perm.denies(action) {
                return false;
            }
        }

        // Then check for allows
        for perm in &self.permissions {
            if !perm.deny && perm.resource.matches(resource) && perm.allows(action) {
                return true;
            }
        }

        // Default deny
        false
    }

    /// Merge another permission set into this one
    pub fn merge(&mut self, other: &PermissionSet) {
        for perm in &other.permissions {
            self.permissions.push(perm.clone());
        }
    }

    /// Get all permissions
    pub fn permissions(&self) -> &[Permission] {
        &self.permissions
    }
}

/// Permission check result for detailed reporting
#[derive(Debug)]
pub struct PermissionCheck {
    /// Resource being accessed
    pub resource: Resource,

    /// Action being attempted
    pub action: Action,

    /// Whether access was granted
    pub granted: bool,

    /// Matching permission (if any)
    pub matching_permission: Option<Permission>,

    /// Reason for decision
    pub reason: String,
}

impl PermissionCheck {
    /// Check permissions and return detailed result
    pub fn check(
        permission_set: &PermissionSet,
        resource: &Resource,
        action: &Action,
    ) -> Self {
        // Check for explicit denials first
        for perm in permission_set.permissions() {
            if perm.deny && perm.resource.matches(resource) && perm.denies(action) {
                return Self {
                    resource: resource.clone(),
                    action: action.clone(),
                    granted: false,
                    matching_permission: Some(perm.clone()),
                    reason: format!("Explicitly denied by permission on {}", perm.resource.to_string()),
                };
            }
        }

        // Check for allows
        for perm in permission_set.permissions() {
            if !perm.deny && perm.resource.matches(resource) && perm.allows(action) {
                return Self {
                    resource: resource.clone(),
                    action: action.clone(),
                    granted: true,
                    matching_permission: Some(perm.clone()),
                    reason: format!("Allowed by permission on {}", perm.resource.to_string()),
                };
            }
        }

        // Default deny
        Self {
            resource: resource.clone(),
            action: action.clone(),
            granted: false,
            matching_permission: None,
            reason: "No matching permission found (default deny)".to_string(),
        }
    }
}

/// Built-in resource types
pub mod resources {
    use super::Resource;

    /// Agent resources
    pub fn agent(name: &str) -> Resource {
        Resource::new("agent", name)
    }

    pub fn all_agents() -> Resource {
        Resource::all("agent")
    }

    /// Service resources
    pub fn service(name: &str) -> Resource {
        Resource::new("service", name)
    }

    pub fn all_services() -> Resource {
        Resource::all("service")
    }

    /// Directory facilitator
    pub fn df() -> Resource {
        Resource::new("platform", "df")
    }

    /// Agent management system
    pub fn ams() -> Resource {
        Resource::new("platform", "ams")
    }

    /// Message resources
    pub fn message(agent: &str) -> Resource {
        Resource::new("message", agent)
    }

    pub fn all_messages() -> Resource {
        Resource::all("message")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_matching() {
        let pattern = Resource::new("agent", "*");
        let specific = Resource::new("agent", "my-agent");

        assert!(pattern.matches(&specific));
        assert!(!specific.matches(&pattern));
    }

    #[test]
    fn test_resource_prefix_matching() {
        let pattern = Resource::new("agent", "test-*");
        let match1 = Resource::new("agent", "test-agent-1");
        let match2 = Resource::new("agent", "test-agent-2");
        let no_match = Resource::new("agent", "prod-agent");

        assert!(pattern.matches(&match1));
        assert!(pattern.matches(&match2));
        assert!(!pattern.matches(&no_match));
    }

    #[test]
    fn test_action_implies() {
        assert!(Action::Admin.implies(&Action::Read));
        assert!(Action::Admin.implies(&Action::Create));
        assert!(!Action::Read.implies(&Action::Create));
    }

    #[test]
    fn test_permission_set_basic() {
        let mut perms = PermissionSet::new();
        perms.allow(
            Resource::new("agent", "my-agent"),
            vec![Action::Read, Action::Update],
        );

        let resource = Resource::new("agent", "my-agent");
        assert!(perms.check(&resource, &Action::Read));
        assert!(perms.check(&resource, &Action::Update));
        assert!(!perms.check(&resource, &Action::Delete));
    }

    #[test]
    fn test_permission_set_wildcard() {
        let mut perms = PermissionSet::new();
        perms.allow(Resource::all("agent"), vec![Action::Read]);

        assert!(perms.check(&Resource::new("agent", "any-agent"), &Action::Read));
        assert!(!perms.check(&Resource::new("service", "svc"), &Action::Read));
    }

    #[test]
    fn test_permission_deny_override() {
        let mut perms = PermissionSet::new();
        // Allow all agent reads
        perms.allow(Resource::all("agent"), vec![Action::Read]);
        // But deny reading "secret-agent"
        perms.deny(
            Resource::new("agent", "secret-agent"),
            vec![Action::Read],
        );

        assert!(perms.check(&Resource::new("agent", "normal-agent"), &Action::Read));
        assert!(!perms.check(&Resource::new("agent", "secret-agent"), &Action::Read));
    }

    #[test]
    fn test_admin_permission() {
        let perms = PermissionSet::admin();

        assert!(perms.check(&Resource::new("agent", "any"), &Action::Create));
        assert!(perms.check(&Resource::new("service", "any"), &Action::Delete));
        assert!(perms.check(&Resource::new("platform", "ams"), &Action::Admin));
    }

    #[test]
    fn test_permission_check_detailed() {
        let mut perms = PermissionSet::new();
        perms.allow(Resource::new("agent", "allowed"), vec![Action::Read]);

        let check1 = PermissionCheck::check(
            &perms,
            &Resource::new("agent", "allowed"),
            &Action::Read,
        );
        assert!(check1.granted);
        assert!(check1.matching_permission.is_some());

        let check2 = PermissionCheck::check(
            &perms,
            &Resource::new("agent", "denied"),
            &Action::Read,
        );
        assert!(!check2.granted);
        assert!(check2.reason.contains("default deny"));
    }
}
