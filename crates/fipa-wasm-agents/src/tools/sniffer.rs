// tools/sniffer.rs - Message Sniffer Agent
//
//! Message Sniffer for FIPA Agent Platforms
//!
//! The sniffer intercepts and records agent messages for debugging,
//! monitoring, and analysis purposes.
//!
//! # Features
//!
//! - Intercept messages for specified agents
//! - Filter by performative, protocol, conversation
//! - Display as sequence diagram
//! - Save/load message traces
//! - Export to JSON/CSV

use crate::proto::{AclMessage, Performative, ProtocolType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use parking_lot::RwLock;
use tracing::{debug, info};

/// Sniffer configuration
#[derive(Debug, Clone)]
pub struct SnifferConfig {
    /// Maximum number of messages to retain
    pub max_messages: usize,

    /// Agents to monitor (empty = all)
    pub monitored_agents: Vec<String>,

    /// Enable sequence diagram output
    pub sequence_diagram: bool,

    /// Auto-export path (optional)
    pub auto_export_path: Option<String>,
}

impl Default for SnifferConfig {
    fn default() -> Self {
        Self {
            max_messages: 10000,
            monitored_agents: vec![],
            sequence_diagram: false,
            auto_export_path: None,
        }
    }
}

/// Filter for sniffer queries
#[derive(Debug, Clone, Default)]
pub struct SnifferFilter {
    /// Filter by sender name (partial match)
    pub sender: Option<String>,

    /// Filter by receiver name (partial match)
    pub receiver: Option<String>,

    /// Filter by performative
    pub performative: Option<Performative>,

    /// Filter by protocol
    pub protocol: Option<ProtocolType>,

    /// Filter by conversation ID
    pub conversation_id: Option<String>,

    /// Filter by time range (start)
    pub from_time: Option<DateTime<Utc>>,

    /// Filter by time range (end)
    pub to_time: Option<DateTime<Utc>>,

    /// Filter by content pattern
    pub content_pattern: Option<String>,
}

/// A single trace entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Unique trace ID
    pub id: u64,

    /// Capture timestamp
    pub timestamp: DateTime<Utc>,

    /// Message ID
    pub message_id: String,

    /// Sender agent name
    pub sender: String,

    /// Receiver agent names
    pub receivers: Vec<String>,

    /// Performative
    pub performative: String,

    /// Protocol (if specified)
    pub protocol: Option<String>,

    /// Conversation ID (if specified)
    pub conversation_id: Option<String>,

    /// Content (text representation)
    pub content: String,

    /// Content length in bytes
    pub content_length: usize,

    /// Direction relative to monitored agent
    pub direction: MessageDirection,
}

/// Message direction relative to a monitored agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageDirection {
    /// Message sent by monitored agent
    Outgoing,
    /// Message received by monitored agent
    Incoming,
    /// Message between other agents (observed)
    Observed,
}

/// A complete message trace
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageTrace {
    /// Trace name/identifier
    pub name: String,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Trace entries
    pub entries: Vec<TraceEntry>,

    /// Agents involved
    pub agents: Vec<String>,

    /// Conversations captured
    pub conversations: Vec<String>,
}

impl MessageTrace {
    /// Create a new empty trace
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            created_at: Utc::now(),
            entries: vec![],
            agents: vec![],
            conversations: vec![],
        }
    }

    /// Add an entry to the trace
    pub fn add_entry(&mut self, entry: TraceEntry) {
        // Track unique agents
        if !self.agents.contains(&entry.sender) {
            self.agents.push(entry.sender.clone());
        }
        for recv in &entry.receivers {
            if !self.agents.contains(recv) {
                self.agents.push(recv.clone());
            }
        }

        // Track conversations
        if let Some(ref conv) = entry.conversation_id {
            if !self.conversations.contains(conv) {
                self.conversations.push(conv.clone());
            }
        }

        self.entries.push(entry);
    }

    /// Export trace to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Import trace from JSON
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Export trace to CSV
    pub fn to_csv(&self) -> String {
        let mut csv = String::new();

        // Header
        csv.push_str("id,timestamp,message_id,sender,receivers,performative,protocol,conversation_id,content_length,direction\n");

        // Rows
        for entry in &self.entries {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{:?}\n",
                entry.id,
                entry.timestamp.to_rfc3339(),
                entry.message_id,
                entry.sender,
                entry.receivers.join(";"),
                entry.performative,
                entry.protocol.as_deref().unwrap_or(""),
                entry.conversation_id.as_deref().unwrap_or(""),
                entry.content_length,
                entry.direction,
            ));
        }

        csv
    }

    /// Generate ASCII sequence diagram
    pub fn to_sequence_diagram(&self) -> String {
        let mut diagram = String::new();

        if self.agents.is_empty() {
            return "No agents in trace".to_string();
        }

        // Calculate column widths and positions
        let col_width = 20;
        let agents = &self.agents;

        // Header with agent names
        diagram.push_str("\n");
        for agent in agents {
            diagram.push_str(&format!("{:^width$}", agent, width = col_width));
        }
        diagram.push_str("\n");

        // Vertical lines
        for _ in agents {
            diagram.push_str(&format!("{:^width$}", "|", width = col_width));
        }
        diagram.push_str("\n");

        // Messages
        for entry in &self.entries {
            let sender_idx = agents.iter().position(|a| a == &entry.sender);
            let receiver_idx = if let Some(first) = entry.receivers.first() {
                agents.iter().position(|a| a == first)
            } else {
                None
            };

            if let (Some(s), Some(r)) = (sender_idx, receiver_idx) {
                let arrow = if s < r {
                    // Left to right
                    format!(
                        "{}--[{}]-->{}",
                        " ".repeat(s * col_width + col_width / 2),
                        &entry.performative,
                        " ".repeat((r - s - 1) * col_width)
                    )
                } else if s > r {
                    // Right to left
                    format!(
                        "{}<--[{}]--{}",
                        " ".repeat(r * col_width + col_width / 2),
                        &entry.performative,
                        " ".repeat((s - r - 1) * col_width)
                    )
                } else {
                    // Self message
                    format!(
                        "{}--[{}]-->|",
                        " ".repeat(s * col_width + col_width / 2),
                        &entry.performative
                    )
                };

                diagram.push_str(&arrow);
                diagram.push_str("\n");

                // Vertical lines
                for _ in agents {
                    diagram.push_str(&format!("{:^width$}", "|", width = col_width));
                }
                diagram.push_str("\n");
            }
        }

        diagram
    }

    /// Get statistics about the trace
    pub fn statistics(&self) -> TraceStatistics {
        let mut stats = TraceStatistics::default();
        stats.total_messages = self.entries.len();
        stats.unique_agents = self.agents.len();
        stats.unique_conversations = self.conversations.len();

        for entry in &self.entries {
            *stats.performatives.entry(entry.performative.clone()).or_insert(0) += 1;
            if let Some(ref proto) = entry.protocol {
                *stats.protocols.entry(proto.clone()).or_insert(0) += 1;
            }
            stats.total_bytes += entry.content_length;
        }

        if let Some(first) = self.entries.first() {
            if let Some(last) = self.entries.last() {
                stats.duration_ms = (last.timestamp - first.timestamp).num_milliseconds();
            }
        }

        stats
    }
}

/// Trace statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceStatistics {
    /// Total messages
    pub total_messages: usize,
    /// Unique agents
    pub unique_agents: usize,
    /// Unique conversations
    pub unique_conversations: usize,
    /// Performative counts
    pub performatives: HashMap<String, usize>,
    /// Protocol counts
    pub protocols: HashMap<String, usize>,
    /// Total content bytes
    pub total_bytes: usize,
    /// Duration in milliseconds
    pub duration_ms: i64,
}

/// Message sniffer
pub struct MessageSniffer {
    /// Configuration
    config: SnifferConfig,

    /// Message buffer
    messages: Arc<RwLock<VecDeque<TraceEntry>>>,

    /// Next trace ID
    next_id: Arc<RwLock<u64>>,

    /// Active sniffing sessions
    sessions: Arc<RwLock<HashMap<String, SnifferFilter>>>,

    /// Statistics
    stats: Arc<RwLock<SnifferStats>>,
}

/// Sniffer statistics
#[derive(Debug, Clone, Default)]
pub struct SnifferStats {
    pub messages_captured: u64,
    pub messages_filtered: u64,
    pub bytes_captured: u64,
}

impl MessageSniffer {
    /// Create a new message sniffer
    pub fn new(config: SnifferConfig) -> Self {
        info!("Creating message sniffer with max {} messages", config.max_messages);
        Self {
            config,
            messages: Arc::new(RwLock::new(VecDeque::new())),
            next_id: Arc::new(RwLock::new(1)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(SnifferStats::default())),
        }
    }

    /// Record a message
    pub fn record_message(&self, message: &AclMessage, direction: MessageDirection) {
        let sender = message
            .sender
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        // Check if we should monitor this message
        if !self.config.monitored_agents.is_empty() {
            let should_monitor = self.config.monitored_agents.iter().any(|a| {
                a == &sender || message.receivers.iter().any(|r| &r.name == a)
            });

            if !should_monitor {
                return;
            }
        }

        let receivers: Vec<String> = message.receivers.iter().map(|r| r.name.clone()).collect();

        let performative = format!("{:?}", Performative::try_from(message.performative).unwrap_or(Performative::Unspecified));

        let protocol = message.protocol.and_then(|p| {
            ProtocolType::try_from(p).ok().map(|pt| format!("{:?}", pt))
        });

        let content = String::from_utf8_lossy(&message.content).to_string();

        let mut id_guard = self.next_id.write();
        let id = *id_guard;
        *id_guard += 1;

        let entry = TraceEntry {
            id,
            timestamp: Utc::now(),
            message_id: message.message_id.clone(),
            sender,
            receivers,
            performative,
            protocol,
            conversation_id: message.conversation_id.clone(),
            content_length: message.content.len(),
            content: if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content
            },
            direction,
        };

        debug!("Sniffer: captured message {} from {} -> {:?}",
            entry.message_id, entry.sender, entry.receivers);

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.messages_captured += 1;
            stats.bytes_captured += message.content.len() as u64;
        }

        // Add to buffer
        let mut messages = self.messages.write();
        messages.push_back(entry);

        // Trim if over limit
        while messages.len() > self.config.max_messages {
            messages.pop_front();
        }
    }

    /// Get captured messages with optional filter
    pub fn get_messages(&self, filter: Option<&SnifferFilter>) -> Vec<TraceEntry> {
        let messages = self.messages.read();

        messages
            .iter()
            .filter(|entry| {
                if let Some(f) = filter {
                    Self::matches_filter(entry, f)
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }

    /// Check if entry matches filter
    fn matches_filter(entry: &TraceEntry, filter: &SnifferFilter) -> bool {
        // Sender filter
        if let Some(ref sender) = filter.sender {
            if !entry.sender.contains(sender) {
                return false;
            }
        }

        // Receiver filter
        if let Some(ref receiver) = filter.receiver {
            if !entry.receivers.iter().any(|r| r.contains(receiver)) {
                return false;
            }
        }

        // Performative filter
        if let Some(ref perf) = filter.performative {
            if entry.performative != format!("{:?}", perf) {
                return false;
            }
        }

        // Protocol filter
        if let Some(ref proto) = filter.protocol {
            let proto_str = format!("{:?}", proto);
            if entry.protocol.as_ref() != Some(&proto_str) {
                return false;
            }
        }

        // Conversation filter
        if let Some(ref conv) = filter.conversation_id {
            if entry.conversation_id.as_ref() != Some(conv) {
                return false;
            }
        }

        // Time range filter
        if let Some(from) = filter.from_time {
            if entry.timestamp < from {
                return false;
            }
        }

        if let Some(to) = filter.to_time {
            if entry.timestamp > to {
                return false;
            }
        }

        // Content pattern filter
        if let Some(ref pattern) = filter.content_pattern {
            if !entry.content.contains(pattern) {
                return false;
            }
        }

        true
    }

    /// Create a trace from captured messages
    pub fn create_trace(&self, name: impl Into<String>, filter: Option<&SnifferFilter>) -> MessageTrace {
        let mut trace = MessageTrace::new(name);
        let messages = self.get_messages(filter);

        for entry in messages {
            trace.add_entry(entry);
        }

        trace
    }

    /// Clear captured messages
    pub fn clear(&self) {
        let mut messages = self.messages.write();
        messages.clear();
        info!("Sniffer: cleared message buffer");
    }

    /// Get sniffer statistics
    pub fn stats(&self) -> SnifferStats {
        self.stats.read().clone()
    }

    /// Start a sniffing session
    pub fn start_session(&self, session_id: String, filter: SnifferFilter) {
        let mut sessions = self.sessions.write();
        sessions.insert(session_id.clone(), filter);
        info!("Sniffer: started session '{}'", session_id);
    }

    /// Stop a sniffing session
    pub fn stop_session(&self, session_id: &str) -> Option<SnifferFilter> {
        let mut sessions = self.sessions.write();
        let filter = sessions.remove(session_id);
        if filter.is_some() {
            info!("Sniffer: stopped session '{}'", session_id);
        }
        filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_creation() {
        let trace = MessageTrace::new("test-trace");
        assert_eq!(trace.name, "test-trace");
        assert!(trace.entries.is_empty());
    }

    #[test]
    fn test_trace_entry() {
        let mut trace = MessageTrace::new("test");

        let entry = TraceEntry {
            id: 1,
            timestamp: Utc::now(),
            message_id: "msg-1".to_string(),
            sender: "agent-a".to_string(),
            receivers: vec!["agent-b".to_string()],
            performative: "Request".to_string(),
            protocol: Some("ProtocolRequest".to_string()),
            conversation_id: Some("conv-1".to_string()),
            content: "Hello".to_string(),
            content_length: 5,
            direction: MessageDirection::Outgoing,
        };

        trace.add_entry(entry);

        assert_eq!(trace.entries.len(), 1);
        assert_eq!(trace.agents.len(), 2);
        assert_eq!(trace.conversations.len(), 1);
    }

    #[test]
    fn test_trace_csv_export() {
        let mut trace = MessageTrace::new("test");

        let entry = TraceEntry {
            id: 1,
            timestamp: Utc::now(),
            message_id: "msg-1".to_string(),
            sender: "agent-a".to_string(),
            receivers: vec!["agent-b".to_string()],
            performative: "Request".to_string(),
            protocol: None,
            conversation_id: None,
            content: "Hello".to_string(),
            content_length: 5,
            direction: MessageDirection::Outgoing,
        };

        trace.add_entry(entry);

        let csv = trace.to_csv();
        assert!(csv.contains("agent-a"));
        assert!(csv.contains("agent-b"));
        assert!(csv.contains("Request"));
    }

    #[test]
    fn test_sniffer_config_default() {
        let config = SnifferConfig::default();
        assert_eq!(config.max_messages, 10000);
        assert!(config.monitored_agents.is_empty());
    }

    #[test]
    fn test_trace_statistics() {
        let mut trace = MessageTrace::new("test");

        for i in 0..5 {
            trace.add_entry(TraceEntry {
                id: i,
                timestamp: Utc::now(),
                message_id: format!("msg-{}", i),
                sender: "agent-a".to_string(),
                receivers: vec!["agent-b".to_string()],
                performative: "Request".to_string(),
                protocol: Some("ProtocolRequest".to_string()),
                conversation_id: Some("conv-1".to_string()),
                content: "Hello".to_string(),
                content_length: 5,
                direction: MessageDirection::Outgoing,
            });
        }

        let stats = trace.statistics();
        assert_eq!(stats.total_messages, 5);
        assert_eq!(stats.unique_agents, 2);
        assert_eq!(stats.unique_conversations, 1);
        assert_eq!(stats.total_bytes, 25);
    }
}
