// acl_message.rs
// Core FIPA ACL message structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent identifier with addressing information
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId {
    pub name: String,
    pub addresses: Vec<String>,
    pub resolvers: Vec<String>,
}

impl AgentId {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            addresses: Vec::new(),
            resolvers: Vec::new(),
        }
    }
}

/// Receiver can be single agent, multiple agents, or broadcast
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReceiverSet {
    Single(AgentId),
    Multiple(Vec<AgentId>),
    Broadcast,
}

impl ReceiverSet {
    pub fn first_agent_id(&self) -> AgentId {
        match self {
            ReceiverSet::Single(id) => id.clone(),
            ReceiverSet::Multiple(ids) => ids.first().unwrap().clone(),
            ReceiverSet::Broadcast => panic!("Cannot get single agent from broadcast"),
        }
    }
}

/// FIPA performative types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Performative {
    AcceptProposal,
    Agree,
    Cancel,
    Cfp,
    Confirm,
    Disconfirm,
    Failure,
    Inform,
    InformDone,
    InformIf,
    InformRef,
    InformResult,
    NotUnderstood,
    Propagate,
    Propose,
    Proxy,
    QueryIf,
    QueryRef,
    Refuse,
    RejectProposal,
    Request,
    RequestWhen,
    RequestWhenever,
    Subscribe,
}

impl Performative {
    pub fn from_i32(value: i32) -> Result<Self, String> {
        match value {
            0 => Ok(Performative::AcceptProposal),
            1 => Ok(Performative::Agree),
            2 => Ok(Performative::Cancel),
            3 => Ok(Performative::Cfp),
            4 => Ok(Performative::Confirm),
            5 => Ok(Performative::Disconfirm),
            6 => Ok(Performative::Failure),
            7 => Ok(Performative::Inform),
            8 => Ok(Performative::InformDone),
            9 => Ok(Performative::InformIf),
            10 => Ok(Performative::InformRef),
            11 => Ok(Performative::InformResult),
            12 => Ok(Performative::NotUnderstood),
            13 => Ok(Performative::Propagate),
            14 => Ok(Performative::Propose),
            15 => Ok(Performative::Proxy),
            16 => Ok(Performative::QueryIf),
            17 => Ok(Performative::QueryRef),
            18 => Ok(Performative::Refuse),
            19 => Ok(Performative::RejectProposal),
            20 => Ok(Performative::Request),
            21 => Ok(Performative::RequestWhen),
            22 => Ok(Performative::RequestWhenever),
            23 => Ok(Performative::Subscribe),
            _ => Err(format!("Unknown performative: {}", value)),
        }
    }
}

/// FIPA protocol types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtocolType {
    Request,
    Query,
    RequestWhen,
    ContractNet,
    IteratedContractNet,
    Propose,
    Brokering,
    Recruiting,
    Subscribe,
    EnglishAuction,
    DutchAuction,
    Custom(String),
}

impl ProtocolType {
    pub fn from_i32(value: i32) -> Result<Self, String> {
        match value {
            0 => Ok(ProtocolType::Request),
            1 => Ok(ProtocolType::Query),
            2 => Ok(ProtocolType::RequestWhen),
            3 => Ok(ProtocolType::ContractNet),
            4 => Ok(ProtocolType::IteratedContractNet),
            5 => Ok(ProtocolType::Propose),
            6 => Ok(ProtocolType::Brokering),
            7 => Ok(ProtocolType::Recruiting),
            8 => Ok(ProtocolType::Subscribe),
            9 => Ok(ProtocolType::EnglishAuction),
            10 => Ok(ProtocolType::DutchAuction),
            _ => Err(format!("Unknown protocol: {}", value)),
        }
    }
}

/// Conversation identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub String);

/// Message identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

/// Timestamp for deadlines and timing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    pub millis: i64,
}

impl Timestamp {
    pub fn now() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");
        Self {
            millis: duration.as_millis() as i64,
        }
    }
}

/// Content language specification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentLanguage {
    FipaSL,
    FipaSL0,
    FipaSL1,
    FipaSL2,
    Xml,
    Rdf,
    Custom(String),
}

/// Content encoding
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    Utf8,
    Base64,
    Custom(String),
}

/// Ontology reference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyRef(pub String);

/// Complete ACL message header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclMessageHeader {
    pub performative: Performative,
    pub sender: AgentId,
    pub receiver: ReceiverSet,
    pub protocol: Option<ProtocolType>,
    pub conversation_id: Option<ConversationId>,
    pub reply_with: Option<MessageId>,
    pub in_reply_to: Option<MessageId>,
    pub reply_by: Option<Timestamp>,
    pub language: Option<ContentLanguage>,
    pub encoding: Option<Encoding>,
    pub ontology: Option<OntologyRef>,
}

/// Message content types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    Binary(Vec<u8>),
    Structured(StructuredContent),
}

/// Structured content for FIPA-SL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredContent {
    pub expressions: Vec<ContentExpression>,
}

/// Content expression types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentExpression {
    Action(String),
    Fact(String),
    Query(String),
    Proposal(String),
}

/// Complete ACL message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclMessage {
    pub header: AclMessageHeader,
    pub content: Option<MessageContent>,
}

impl AclMessage {
    pub fn new(
        performative: Performative,
        sender: AgentId,
        receiver: ReceiverSet,
    ) -> Self {
        Self {
            header: AclMessageHeader {
                performative,
                sender,
                receiver,
                protocol: None,
                conversation_id: None,
                reply_with: None,
                in_reply_to: None,
                reply_by: None,
                language: Some(ContentLanguage::FipaSL),
                encoding: Some(Encoding::Utf8),
                ontology: None,
            },
            content: None,
        }
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(MessageContent::Text(content.into()));
        self
    }

    pub fn with_protocol(mut self, protocol: ProtocolType) -> Self {
        self.header.protocol = Some(protocol);
        self
    }

    pub fn with_conversation(mut self, conv_id: ConversationId) -> Self {
        self.header.conversation_id = Some(conv_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let sender = AgentId::new("agent1");
        let receiver = ReceiverSet::Single(AgentId::new("agent2"));
        
        let msg = AclMessage::new(Performative::Request, sender, receiver)
            .with_content("perform action X")
            .with_protocol(ProtocolType::Request);
        
        assert_eq!(msg.header.performative, Performative::Request);
        assert!(matches!(msg.content, Some(MessageContent::Text(_))));
    }
}
