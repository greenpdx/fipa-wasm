// protocol/query.rs - FIPA Query Protocol

use super::state_machine::*;
use crate::proto;

/// Query type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryType {
    /// Boolean query (query-if)
    If,
    /// Reference query (query-ref)
    Ref,
}

/// FIPA Query Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryState {
    NotStarted,
    Queried,
    Agreed,
    Completed,
    Failed,
    Refused,
}

impl QueryState {
    pub fn as_str(&self) -> &'static str {
        match self {
            QueryState::NotStarted => "not_started",
            QueryState::Queried => "queried",
            QueryState::Agreed => "agreed",
            QueryState::Completed => "completed",
            QueryState::Failed => "failed",
            QueryState::Refused => "refused",
        }
    }
}

/// FIPA Query Protocol Implementation
#[derive(Debug)]
pub struct QueryProtocol {
    state: QueryState,
    base: ConversationBase,
    query_type: Option<QueryType>,
    query_content: Option<Vec<u8>>,
    result: Option<Vec<u8>>,
}

impl QueryProtocol {
    pub fn new(role: Role) -> Self {
        Self {
            state: QueryState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), role),
            query_type: None,
            query_content: None,
            result: None,
        }
    }

    fn validate_transition(&self, performative: proto::Performative) -> Result<QueryState, ProtocolError> {
        use proto::Performative::*;
        use QueryState::*;

        match (&self.state, performative) {
            (NotStarted, QueryIf) | (NotStarted, QueryRef) => Ok(Queried),
            (Queried, Agree) => Ok(Agreed),
            (Queried, Refuse) => Ok(Refused),
            (Agreed, InformIf) | (Agreed, InformRef) | (Agreed, InformResult) => Ok(Completed),
            (Agreed, Failure) => Ok(Failed),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for QueryProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolQuery
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
            proto::Performative::QueryIf => {
                self.query_type = Some(QueryType::If);
                self.query_content = Some(msg.content.clone());
            }
            proto::Performative::QueryRef => {
                self.query_type = Some(QueryType::Ref);
                self.query_content = Some(msg.content.clone());
            }
            proto::Performative::InformIf
            | proto::Performative::InformRef
            | proto::Performative::InformResult => {
                self.result = Some(msg.content.clone());
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            QueryState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: self.result.clone(),
                ..Default::default()
            })),
            QueryState::Failed | QueryState::Refused => {
                Ok(ProcessResult::Failed("Query failed".into()))
            }
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            QueryState::Completed | QueryState::Failed | QueryState::Refused
        )
    }

    fn is_failed(&self) -> bool {
        matches!(self.state, QueryState::Failed | QueryState::Refused)
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            QueryState::NotStarted => vec![QueryIf, QueryRef],
            QueryState::Queried => vec![Agree, Refuse],
            QueryState::Agreed => vec![InformIf, InformRef, InformResult, Failure],
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
