// protocol/recruiting.rs - FIPA Recruiting Protocol
//
//! FIPA Recruiting Protocol Implementation
//!
//! The Recruiting protocol is similar to brokering, but the recruiter
//! helps the initiator find suitable agents rather than forwarding
//! requests. The initiator then communicates directly with the
//! discovered agents.
//!
//! # Protocol Flow
//!
//! ```text
//! Initiator         Recruiter          Directory
//!     |                |                    |
//!     |--- PROXY ----->|                    |
//!     |                |                    |
//!     |                |--- QUERY-REF ----->|
//!     |                |                    |
//!     |                |<-- INFORM ---------|
//!     |                |                    |
//!     |<-- INFORM -----|                    |
//!     | (list of agents)                    |
//!     |                                     |
//!     |--------------- REQUEST ------------>| (direct to agent)
//!     |                                     |
//! ```

use super::state_machine::*;
use crate::proto;

/// Recruiting Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecruitingState {
    /// Initial state
    NotStarted,
    /// Proxy request received
    ProxyReceived,
    /// Searching for candidates
    Searching,
    /// Candidates found, informing initiator
    CandidatesFound,
    /// Completed successfully
    Completed,
    /// No candidates found
    NoCandidates,
    /// Failed
    Failed,
    /// Cancelled
    Cancelled,
}

impl RecruitingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecruitingState::NotStarted => "not_started",
            RecruitingState::ProxyReceived => "proxy_received",
            RecruitingState::Searching => "searching",
            RecruitingState::CandidatesFound => "candidates_found",
            RecruitingState::Completed => "completed",
            RecruitingState::NoCandidates => "no_candidates",
            RecruitingState::Failed => "failed",
            RecruitingState::Cancelled => "cancelled",
        }
    }
}

/// Candidate agent information
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Agent ID
    pub agent_id: proto::AgentId,
    /// Service description
    pub service: Option<proto::ServiceDescription>,
    /// Ranking score
    pub score: f64,
}

/// Recruiting Protocol Implementation
#[derive(Debug)]
pub struct RecruitingProtocol {
    /// Current state
    state: RecruitingState,

    /// Conversation base
    base: ConversationBase,

    /// Original request from initiator
    original_request: Option<proto::AclMessage>,

    /// Initiator agent ID
    initiator: Option<proto::AgentId>,

    /// Search criteria
    search_criteria: Option<Vec<u8>>,

    /// Service name to search for
    service_name: Option<String>,

    /// Protocol requirement
    required_protocol: Option<proto::ProtocolType>,

    /// Found candidates
    candidates: Vec<Candidate>,

    /// Maximum candidates to return
    max_candidates: usize,

    /// Minimum score threshold
    min_score: f64,
}

impl RecruitingProtocol {
    /// Create a new recruiting protocol (as recruiter)
    pub fn new_as_recruiter() -> Self {
        Self {
            state: RecruitingState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            original_request: None,
            initiator: None,
            search_criteria: None,
            service_name: None,
            required_protocol: None,
            candidates: vec![],
            max_candidates: 10,
            min_score: 0.0,
        }
    }

    /// Create a new recruiting protocol (as initiator)
    pub fn new_as_initiator() -> Self {
        Self {
            state: RecruitingState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            original_request: None,
            initiator: None,
            search_criteria: None,
            service_name: None,
            required_protocol: None,
            candidates: vec![],
            max_candidates: 10,
            min_score: 0.0,
        }
    }

    /// Set service name to search for
    pub fn with_service_name(mut self, name: String) -> Self {
        self.service_name = Some(name);
        self
    }

    /// Set required protocol
    pub fn with_required_protocol(mut self, protocol: proto::ProtocolType) -> Self {
        self.required_protocol = Some(protocol);
        self
    }

    /// Set maximum candidates
    pub fn with_max_candidates(mut self, count: usize) -> Self {
        self.max_candidates = count;
        self
    }

    /// Set minimum score
    pub fn with_min_score(mut self, score: f64) -> Self {
        self.min_score = score;
        self
    }

    /// Set conversation ID
    pub fn with_conversation_id(mut self, id: String) -> Self {
        self.base.conversation_id = id;
        self
    }

    /// Add a candidate
    pub fn add_candidate(&mut self, agent_id: proto::AgentId, service: Option<proto::ServiceDescription>, score: f64) {
        if score >= self.min_score && self.candidates.len() < self.max_candidates {
            self.candidates.push(Candidate {
                agent_id,
                service,
                score,
            });
            // Sort by score descending
            self.candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }
    }

    /// Get candidates
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// Complete the search phase
    pub fn complete_search(&mut self) -> Result<(), ProtocolError> {
        if self.state != RecruitingState::Searching {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "complete_search".to_string(),
            });
        }

        if self.candidates.is_empty() {
            self.state = RecruitingState::NoCandidates;
        } else {
            self.state = RecruitingState::CandidatesFound;
        }

        Ok(())
    }

    /// Validate state transition
    fn validate_transition(&self, performative: proto::Performative) -> Result<RecruitingState, ProtocolError> {
        use proto::Performative::*;
        use RecruitingState::*;

        match (&self.state, performative) {
            // Initiator sends proxy request
            (NotStarted, Proxy) => Ok(ProxyReceived),
            // Recruiter queries directory
            (ProxyReceived, QueryRef) => Ok(Searching),
            // Directory responds
            (Searching, Inform) | (Searching, InformRef) => Ok(CandidatesFound),
            // Recruiter informs initiator of candidates
            (CandidatesFound, Inform) => Ok(Completed),
            // No candidates found
            (Searching, Failure) => Ok(NoCandidates),
            // Cancel
            (_, Cancel) => Ok(Cancelled),
            // General failure
            (_, Failure) => Ok(Failed),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for RecruitingProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolRecruiting
    }

    fn state_name(&self) -> &str {
        self.state.as_str()
    }

    fn validate(&self, msg: &proto::AclMessage) -> Result<(), ProtocolError> {
        let performative = proto::Performative::try_from(msg.performative)
            .map_err(|_| ProtocolError::ValidationFailed("Invalid performative".into()))?;

        self.validate_transition(performative)?;
        Ok(())
    }

    fn process(&mut self, msg: proto::AclMessage) -> Result<ProcessResult, ProtocolError> {
        let performative = proto::Performative::try_from(msg.performative)
            .map_err(|_| ProtocolError::ValidationFailed("Invalid performative".into()))?;

        let new_state = self.validate_transition(performative)?;

        self.base.record_message(msg.clone());

        match performative {
            proto::Performative::Proxy => {
                self.original_request = Some(msg.clone());
                self.initiator = msg.sender.clone();
                self.search_criteria = Some(msg.content.clone());
            }
            proto::Performative::Inform | proto::Performative::InformRef => {
                // Parse candidate list from content if available
                // In a real implementation, this would deserialize the content
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            RecruitingState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: Some(serde_json::to_vec(&self.candidates.iter().map(|c| c.agent_id.name.clone()).collect::<Vec<_>>()).unwrap_or_default()),
                ..Default::default()
            })),
            RecruitingState::NoCandidates => Ok(ProcessResult::Failed("No candidates found".into())),
            RecruitingState::Failed => Ok(ProcessResult::Failed("Recruiting failed".into())),
            RecruitingState::Cancelled => Ok(ProcessResult::Failed("Recruiting cancelled".into())),
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            RecruitingState::Completed | RecruitingState::NoCandidates | RecruitingState::Failed | RecruitingState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            RecruitingState::NoCandidates | RecruitingState::Failed | RecruitingState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            RecruitingState::NotStarted => vec![Proxy],
            RecruitingState::ProxyReceived => vec![QueryRef, Cancel],
            RecruitingState::Searching => vec![Inform, InformRef, Failure, Cancel],
            RecruitingState::CandidatesFound => vec![Inform, Cancel],
            _ => vec![],
        }
    }

    fn serialize_state(&self) -> Result<Vec<u8>, ProtocolError> {
        Ok(self.state.as_str().as_bytes().to_vec())
    }

    fn message_history(&self) -> &[proto::AclMessage] {
        &self.base.messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recruiting_basics() {
        let protocol = RecruitingProtocol::new_as_recruiter();
        assert_eq!(protocol.state, RecruitingState::NotStarted);
    }

    #[test]
    fn test_add_candidates() {
        let mut protocol = RecruitingProtocol::new_as_recruiter()
            .with_max_candidates(5)
            .with_min_score(0.5);

        protocol.add_candidate(
            proto::AgentId {
                name: "agent1".into(),
                addresses: vec![],
                resolvers: vec![],
            },
            None,
            0.9,
        );

        protocol.add_candidate(
            proto::AgentId {
                name: "agent2".into(),
                addresses: vec![],
                resolvers: vec![],
            },
            None,
            0.3, // Below threshold
        );

        assert_eq!(protocol.candidates().len(), 1);
        assert_eq!(protocol.candidates()[0].agent_id.name, "agent1");
    }
}
