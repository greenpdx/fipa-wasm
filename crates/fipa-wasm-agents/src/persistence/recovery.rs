// persistence/recovery.rs - Platform Recovery Engine
//
//! Platform recovery from persisted state
//!
//! Provides:
//! - Agent state restoration
//! - Conversation resumption
//! - Service registry recovery
//! - Recovery validation

use super::snapshot::{AgentSnapshot, ConversationSnapshot, PlatformSnapshot, ServiceSnapshot};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Recovery errors
#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("Agent recovery failed for '{agent}': {reason}")]
    AgentRecoveryFailed { agent: String, reason: String },

    #[error("Conversation recovery failed: {0}")]
    ConversationRecoveryFailed(String),

    #[error("Invalid snapshot data: {0}")]
    InvalidSnapshot(String),

    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u32, actual: u32 },

    #[error("Missing required data: {0}")]
    MissingData(String),

    #[error("Conflict detected: {0}")]
    Conflict(String),
}

/// Result type for recovery operations
pub type RecoveryResult<T> = Result<T, RecoveryError>;

/// Recovery state - the result of platform recovery
#[derive(Debug, Clone, Default)]
pub struct RecoveryState {
    /// Platform metadata
    pub platform: Option<RecoveredPlatform>,

    /// Recovered agents
    pub agents: Vec<RecoveredAgent>,

    /// Recovered services
    pub services: Vec<ServiceSnapshot>,

    /// Recovered conversations
    pub conversations: Vec<RecoveredConversation>,

    /// Recovery warnings
    pub warnings: Vec<String>,

    /// Recovery timestamp
    pub recovered_at: u64,
}

impl RecoveryState {
    /// Create an empty recovery state
    pub fn empty() -> Self {
        Self {
            platform: None,
            agents: vec![],
            services: vec![],
            conversations: vec![],
            warnings: vec![],
            recovered_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Check if there's anything to recover
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.services.is_empty() && self.platform.is_none()
    }

    /// Get agent count
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Get service count
    pub fn service_count(&self) -> usize {
        self.services.len()
    }
}

/// Recovered platform metadata
#[derive(Debug, Clone)]
pub struct RecoveredPlatform {
    /// Platform name
    pub name: String,

    /// Node ID
    pub node_id: Option<String>,

    /// Original snapshot timestamp
    pub snapshot_time: u64,

    /// Configuration
    pub config: HashMap<String, String>,
}

/// Recovered agent data
#[derive(Debug, Clone)]
pub struct RecoveredAgent {
    /// Agent name
    pub name: String,

    /// WASM state to restore
    pub wasm_state: Vec<u8>,

    /// Snapshot ID used for recovery
    pub from_snapshot: String,

    /// Snapshot timestamp
    pub snapshot_time: u64,

    /// Active conversations to resume
    pub conversations: Vec<ConversationSnapshot>,

    /// Pending messages
    pub pending_messages: Vec<PendingMessage>,

    /// Recovery status
    pub status: RecoveryStatus,
}

/// Pending message for delivery after recovery
#[derive(Debug, Clone)]
pub struct PendingMessage {
    /// Message ID
    pub id: String,

    /// Sender
    pub sender: String,

    /// Performative
    pub performative: String,

    /// Content
    pub content: Vec<u8>,

    /// Conversation ID
    pub conversation_id: Option<String>,
}

/// Recovered conversation
#[derive(Debug, Clone)]
pub struct RecoveredConversation {
    /// Conversation ID
    pub id: String,

    /// Protocol
    pub protocol: String,

    /// Current state
    pub state: String,

    /// Participants
    pub participants: Vec<String>,

    /// Can resume (or needs restart)
    pub can_resume: bool,

    /// Reason if can't resume
    pub resume_issue: Option<String>,
}

/// Recovery status for an agent
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryStatus {
    /// Successfully recovered
    Success,
    /// Recovered with warnings
    Partial(Vec<String>),
    /// Recovery failed
    Failed(String),
    /// Skipped (no snapshot available)
    Skipped,
}

/// Recovery engine - builds recovery state from snapshots
#[derive(Debug, Default)]
pub struct RecoveryEngine {
    /// Platform state
    platform_state: Option<PlatformSnapshot>,

    /// Agent snapshots
    agent_snapshots: HashMap<String, AgentSnapshot>,

    /// Services to restore
    services: Vec<ServiceSnapshot>,

    /// Validation enabled
    validate: bool,
}

impl RecoveryEngine {
    /// Create a new recovery engine
    pub fn new() -> Self {
        Self {
            platform_state: None,
            agent_snapshots: HashMap::new(),
            services: vec![],
            validate: true,
        }
    }

    /// Disable validation (for testing)
    pub fn without_validation(mut self) -> Self {
        self.validate = false;
        self
    }

    /// Set platform state
    pub fn set_platform_state(&mut self, state: PlatformSnapshot) {
        self.platform_state = Some(state);
    }

    /// Add an agent snapshot
    pub fn add_agent_snapshot(&mut self, snapshot: AgentSnapshot) {
        self.agent_snapshots
            .insert(snapshot.agent_name.clone(), snapshot);
    }

    /// Add services
    pub fn add_services(&mut self, services: Vec<ServiceSnapshot>) {
        self.services.extend(services);
    }

    /// Build the recovery state
    pub fn build_recovery_state(&self) -> RecoveryResult<RecoveryState> {
        let mut state = RecoveryState::empty();

        // Recover platform
        if let Some(ref platform) = self.platform_state {
            state.platform = Some(RecoveredPlatform {
                name: platform.platform_name.clone(),
                node_id: platform.node_id.clone(),
                snapshot_time: platform.created_at,
                config: platform.config.clone(),
            });
        }

        // Recover agents
        for (name, snapshot) in &self.agent_snapshots {
            match self.recover_agent(snapshot) {
                Ok(agent) => {
                    // Also collect conversations
                    for conv in &agent.conversations {
                        state.conversations.push(self.analyze_conversation(conv));
                    }
                    state.agents.push(agent);
                }
                Err(e) => {
                    warn!("Failed to recover agent '{}': {}", name, e);
                    state.warnings.push(format!("Agent '{}': {}", name, e));
                }
            }
        }

        // Add services
        state.services = self.services.clone();

        info!(
            "Recovery state built: {} agents, {} services, {} warnings",
            state.agents.len(),
            state.services.len(),
            state.warnings.len()
        );

        Ok(state)
    }

    /// Recover a single agent from snapshot
    fn recover_agent(&self, snapshot: &AgentSnapshot) -> RecoveryResult<RecoveredAgent> {
        // Validate snapshot if enabled
        if self.validate {
            self.validate_agent_snapshot(snapshot)?;
        }

        let mut warnings = vec![];

        // Check for stale conversations
        for conv in &snapshot.conversations {
            if self.is_conversation_stale(conv) {
                warnings.push(format!("Conversation '{}' may be stale", conv.conversation_id));
            }
        }

        // Convert pending messages
        let pending_messages: Vec<PendingMessage> = snapshot
            .pending_messages
            .iter()
            .map(|m| PendingMessage {
                id: m.message_id.clone(),
                sender: m.sender.clone(),
                performative: m.performative.clone(),
                content: m.content.clone(),
                conversation_id: m.conversation_id.clone(),
            })
            .collect();

        let status = if warnings.is_empty() {
            RecoveryStatus::Success
        } else {
            RecoveryStatus::Partial(warnings)
        };

        Ok(RecoveredAgent {
            name: snapshot.agent_name.clone(),
            wasm_state: snapshot.wasm_state.clone(),
            from_snapshot: snapshot.id.to_string(),
            snapshot_time: snapshot.metadata.created_at,
            conversations: snapshot.conversations.clone(),
            pending_messages,
            status,
        })
    }

    /// Validate an agent snapshot
    fn validate_agent_snapshot(&self, snapshot: &AgentSnapshot) -> RecoveryResult<()> {
        if snapshot.agent_name.is_empty() {
            return Err(RecoveryError::InvalidSnapshot(
                "Agent name is empty".to_string(),
            ));
        }

        if snapshot.wasm_state.is_empty() {
            debug!(
                "Agent '{}' has empty WASM state (may be intentional)",
                snapshot.agent_name
            );
        }

        // Verify metadata
        if !snapshot.metadata.verify_checksum(&snapshot.wasm_state) {
            return Err(RecoveryError::InvalidSnapshot(
                "Checksum verification failed".to_string(),
            ));
        }

        Ok(())
    }

    /// Check if a conversation is stale
    fn is_conversation_stale(&self, conv: &ConversationSnapshot) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Consider stale if no activity for 1 hour
        const STALE_THRESHOLD: u64 = 3600;
        now - conv.last_activity > STALE_THRESHOLD
    }

    /// Analyze a conversation for recovery
    fn analyze_conversation(&self, conv: &ConversationSnapshot) -> RecoveredConversation {
        let is_stale = self.is_conversation_stale(conv);
        let is_terminal = self.is_terminal_state(&conv.state);

        let (can_resume, issue) = if is_terminal {
            (false, Some("Conversation already completed".to_string()))
        } else if is_stale {
            (false, Some("Conversation timed out".to_string()))
        } else {
            (true, None)
        };

        RecoveredConversation {
            id: conv.conversation_id.clone(),
            protocol: conv.protocol.clone(),
            state: conv.state.clone(),
            participants: conv.participants.clone(),
            can_resume,
            resume_issue: issue,
        }
    }

    /// Check if a state is terminal
    fn is_terminal_state(&self, state: &str) -> bool {
        let terminal_states = [
            "completed",
            "failed",
            "cancelled",
            "done",
            "finished",
            "rejected",
            "refused",
        ];

        terminal_states.iter().any(|s| state.to_lowercase().contains(s))
    }

    /// Get recovery summary
    pub fn summary(&self) -> RecoverySummary {
        RecoverySummary {
            has_platform: self.platform_state.is_some(),
            agent_count: self.agent_snapshots.len(),
            service_count: self.services.len(),
            total_conversations: self
                .agent_snapshots
                .values()
                .map(|s| s.conversations.len())
                .sum(),
        }
    }
}

/// Recovery summary
#[derive(Debug, Clone)]
pub struct RecoverySummary {
    /// Has platform state
    pub has_platform: bool,

    /// Number of agents to recover
    pub agent_count: usize,

    /// Number of services to recover
    pub service_count: usize,

    /// Total conversations across all agents
    pub total_conversations: usize,
}

/// Recovery plan - what will be recovered
#[derive(Debug, Clone)]
pub struct RecoveryPlan {
    /// Agents to recover
    pub agents: Vec<AgentRecoveryPlan>,

    /// Services to restore
    pub services: Vec<String>,

    /// Estimated recovery time (placeholder)
    pub estimated_steps: usize,
}

/// Plan for recovering a single agent
#[derive(Debug, Clone)]
pub struct AgentRecoveryPlan {
    /// Agent name
    pub name: String,

    /// Snapshot to use
    pub snapshot_id: String,

    /// Whether to restore conversations
    pub restore_conversations: bool,

    /// Whether to deliver pending messages
    pub deliver_pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_state_empty() {
        let state = RecoveryState::empty();
        assert!(state.is_empty());
        assert_eq!(state.agent_count(), 0);
    }

    #[test]
    fn test_recovery_engine_basic() {
        let mut engine = RecoveryEngine::new();

        engine.set_platform_state(PlatformSnapshot::new("test-platform"));
        engine.add_agent_snapshot(AgentSnapshot::new("agent1", vec![1, 2, 3]));
        engine.add_services(vec![ServiceSnapshot::new("svc1", "agent1")]);

        let summary = engine.summary();
        assert!(summary.has_platform);
        assert_eq!(summary.agent_count, 1);
        assert_eq!(summary.service_count, 1);
    }

    #[test]
    fn test_build_recovery_state() {
        let mut engine = RecoveryEngine::new().without_validation();

        engine.add_agent_snapshot(AgentSnapshot::new("agent1", vec![1, 2, 3]));
        engine.add_agent_snapshot(AgentSnapshot::new("agent2", vec![4, 5, 6]));

        let state = engine.build_recovery_state().unwrap();

        assert_eq!(state.agents.len(), 2);
        assert!(state.warnings.is_empty());
    }

    #[test]
    fn test_recovered_agent_status() {
        let mut engine = RecoveryEngine::new().without_validation();

        let snapshot = AgentSnapshot::new("test-agent", vec![1, 2, 3]);
        engine.add_agent_snapshot(snapshot);

        let state = engine.build_recovery_state().unwrap();
        let agent = &state.agents[0];

        assert_eq!(agent.status, RecoveryStatus::Success);
    }

    #[test]
    fn test_terminal_state_detection() {
        let engine = RecoveryEngine::new();

        assert!(engine.is_terminal_state("completed"));
        assert!(engine.is_terminal_state("FAILED"));
        assert!(engine.is_terminal_state("task_cancelled"));
        assert!(!engine.is_terminal_state("awaiting_response"));
        assert!(!engine.is_terminal_state("in_progress"));
    }

    #[test]
    fn test_conversation_analysis() {
        let engine = RecoveryEngine::new();

        // Non-terminal, recent conversation
        let conv = ConversationSnapshot::new("conv-1", "request")
            .with_state("awaiting_response");

        let analysis = engine.analyze_conversation(&conv);
        assert!(analysis.can_resume);
        assert!(analysis.resume_issue.is_none());

        // Terminal conversation
        let conv2 = ConversationSnapshot::new("conv-2", "request")
            .with_state("completed");

        let analysis2 = engine.analyze_conversation(&conv2);
        assert!(!analysis2.can_resume);
        assert!(analysis2.resume_issue.is_some());
    }

    #[test]
    fn test_validation_empty_name() {
        let engine = RecoveryEngine::new();

        let mut snapshot = AgentSnapshot::new("", vec![]);
        snapshot.agent_name = "".to_string();

        let result = engine.validate_agent_snapshot(&snapshot);
        assert!(result.is_err());
    }
}
