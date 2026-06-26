// protocol/english_auction.rs - FIPA English Auction Protocol
//
//! FIPA English Auction Protocol Implementation
//!
//! The English Auction is an ascending-price auction where:
//! - Auctioneer announces an item with a starting price
//! - Bidders submit progressively higher bids
//! - Auction ends when no higher bid is received
//! - Highest bidder wins
//!
//! # Protocol Flow
//!
//! ```text
//! Auctioneer                    Bidders
//!     |                            |
//!     |--------- INFORM ---------->|  (auction start, item details)
//!     |                            |
//!     |<-------- PROPOSE ----------|  (bid from bidder 1)
//!     |                            |
//!     |--------- ACCEPT-PROPOSAL ->|  (to bidder 1: bid accepted)
//!     |--------- INFORM ---------->|  (to all: new current price)
//!     |                            |
//!     |<-------- PROPOSE ----------|  (higher bid from bidder 2)
//!     |                            |
//!     |--------- REJECT-PROPOSAL ->|  (to bidder 1: outbid)
//!     |--------- ACCEPT-PROPOSAL ->|  (to bidder 2: bid accepted)
//!     |                            |
//!     |  ... bidding continues ... |
//!     |                            |
//!     |--------- INFORM ---------->|  (auction closed, winner announced)
//!     |                            |
//! ```

use super::state_machine::*;
use crate::proto;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bid information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bid {
    /// Bidder agent name
    pub bidder: String,
    /// Bid amount
    pub amount: f64,
    /// Timestamp
    pub timestamp: i64,
}

/// English Auction Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnglishAuctionState {
    /// Initial state
    NotStarted,
    /// Auction has been announced
    Announced,
    /// Bidding is active
    Bidding,
    /// Auction is closing (no new bids in timeout period)
    Closing,
    /// Auction completed with a winner
    Completed,
    /// Auction failed (no bids or error)
    Failed,
    /// Auction was cancelled
    Cancelled,
}

impl EnglishAuctionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnglishAuctionState::NotStarted => "not_started",
            EnglishAuctionState::Announced => "announced",
            EnglishAuctionState::Bidding => "bidding",
            EnglishAuctionState::Closing => "closing",
            EnglishAuctionState::Completed => "completed",
            EnglishAuctionState::Failed => "failed",
            EnglishAuctionState::Cancelled => "cancelled",
        }
    }
}

/// English Auction Protocol Implementation
#[derive(Debug)]
pub struct EnglishAuctionProtocol {
    /// Current state
    state: EnglishAuctionState,

    /// Conversation base
    base: ConversationBase,

    /// Item being auctioned
    item_description: Option<Vec<u8>>,

    /// Starting price
    starting_price: f64,

    /// Reserve price (minimum acceptable)
    reserve_price: Option<f64>,

    /// Minimum bid increment
    bid_increment: f64,

    /// Current highest bid
    current_bid: Option<Bid>,

    /// All bids received
    bid_history: Vec<Bid>,

    /// Registered bidders
    bidders: HashMap<String, proto::AgentId>,

    /// Winner (if auction completed)
    winner: Option<String>,
}

impl EnglishAuctionProtocol {
    /// Create a new English auction (as auctioneer)
    pub fn new_as_auctioneer(starting_price: f64, bid_increment: f64) -> Self {
        Self {
            state: EnglishAuctionState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            item_description: None,
            starting_price,
            reserve_price: None,
            bid_increment,
            current_bid: None,
            bid_history: vec![],
            bidders: HashMap::new(),
            winner: None,
        }
    }

    /// Create a new English auction (as bidder)
    pub fn new_as_bidder() -> Self {
        Self {
            state: EnglishAuctionState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Participant),
            item_description: None,
            starting_price: 0.0,
            reserve_price: None,
            bid_increment: 0.0,
            current_bid: None,
            bid_history: vec![],
            bidders: HashMap::new(),
            winner: None,
        }
    }

    /// Set reserve price
    pub fn with_reserve_price(mut self, price: f64) -> Self {
        self.reserve_price = Some(price);
        self
    }

    /// Set item description
    pub fn with_item_description(mut self, desc: Vec<u8>) -> Self {
        self.item_description = Some(desc);
        self
    }

    /// Set conversation ID
    pub fn with_conversation_id(mut self, id: String) -> Self {
        self.base.conversation_id = id;
        self
    }

    /// Get current bid
    pub fn current_bid(&self) -> Option<&Bid> {
        self.current_bid.as_ref()
    }

    /// Get minimum acceptable bid
    pub fn minimum_bid(&self) -> f64 {
        self.current_bid
            .as_ref()
            .map(|b| b.amount + self.bid_increment)
            .unwrap_or(self.starting_price)
    }

    /// Get winner
    pub fn winner(&self) -> Option<&str> {
        self.winner.as_deref()
    }

    /// Register a bidder
    pub fn register_bidder(&mut self, agent_id: proto::AgentId) {
        self.bidders.insert(agent_id.name.clone(), agent_id);
    }

    /// Submit a bid (returns true if accepted)
    pub fn submit_bid(&mut self, bidder: &str, amount: f64) -> Result<bool, ProtocolError> {
        if !matches!(self.state, EnglishAuctionState::Announced | EnglishAuctionState::Bidding) {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "bid".to_string(),
            });
        }

        let min_bid = self.minimum_bid();
        if amount < min_bid {
            return Ok(false);
        }

        let bid = Bid {
            bidder: bidder.to_string(),
            amount,
            timestamp: chrono::Utc::now().timestamp_millis(),
        };

        self.bid_history.push(bid.clone());
        self.current_bid = Some(bid);
        self.state = EnglishAuctionState::Bidding;

        Ok(true)
    }

    /// Close the auction
    pub fn close_auction(&mut self) -> Result<Option<&Bid>, ProtocolError> {
        if !matches!(self.state, EnglishAuctionState::Bidding | EnglishAuctionState::Closing) {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "close".to_string(),
            });
        }

        // Check reserve price
        if let (Some(reserve), Some(bid)) = (&self.reserve_price, &self.current_bid) {
            if bid.amount < *reserve {
                self.state = EnglishAuctionState::Failed;
                return Ok(None);
            }
        }

        if let Some(bid) = &self.current_bid {
            self.winner = Some(bid.bidder.clone());
            self.state = EnglishAuctionState::Completed;
            Ok(self.current_bid.as_ref())
        } else {
            self.state = EnglishAuctionState::Failed;
            Ok(None)
        }
    }

    /// Validate state transition based on performative
    fn validate_transition(&self, performative: proto::Performative) -> Result<EnglishAuctionState, ProtocolError> {
        use proto::Performative::*;
        use EnglishAuctionState::*;

        match (&self.state, performative) {
            // Auctioneer announces auction
            (NotStarted, Inform) => Ok(Announced),
            // Bidder proposes a bid
            (Announced, Propose) | (Bidding, Propose) => Ok(Bidding),
            // Auctioneer accepts/rejects bid
            (Bidding, AcceptProposal) | (Bidding, RejectProposal) => Ok(Bidding),
            // Auctioneer announces closing or completion
            (Bidding, Inform) => Ok(Completed),
            (Announced, Inform) => Ok(Completed), // No bids case
            // Failure
            (_, Failure) => Ok(Failed),
            // Cancel
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for EnglishAuctionProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolEnglishAuction
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
            proto::Performative::Inform if self.state == EnglishAuctionState::NotStarted => {
                self.item_description = Some(msg.content.clone());
            }
            proto::Performative::Propose => {
                if let Some(sender) = &msg.sender {
                    self.register_bidder(sender.clone());
                }
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            EnglishAuctionState::Completed => Ok(ProcessResult::Complete(CompletionData {
                result: self.current_bid.as_ref().map(|b| {
                    serde_json::to_vec(&b).unwrap_or_default()
                }),
                ..Default::default()
            })),
            EnglishAuctionState::Failed => Ok(ProcessResult::Failed("Auction failed".into())),
            EnglishAuctionState::Cancelled => Ok(ProcessResult::Failed("Auction cancelled".into())),
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            EnglishAuctionState::Completed | EnglishAuctionState::Failed | EnglishAuctionState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            EnglishAuctionState::Failed | EnglishAuctionState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            EnglishAuctionState::NotStarted => vec![Inform],
            EnglishAuctionState::Announced => vec![Propose, Inform, Cancel],
            EnglishAuctionState::Bidding => vec![Propose, AcceptProposal, RejectProposal, Inform, Cancel],
            EnglishAuctionState::Closing => vec![Inform, Cancel],
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
    fn test_english_auction_basics() {
        let mut auction = EnglishAuctionProtocol::new_as_auctioneer(100.0, 10.0);
        assert_eq!(auction.state, EnglishAuctionState::NotStarted);
        assert_eq!(auction.minimum_bid(), 100.0);
    }

    #[test]
    fn test_bidding() {
        let mut auction = EnglishAuctionProtocol::new_as_auctioneer(100.0, 10.0);
        auction.state = EnglishAuctionState::Announced;

        // Valid bid
        assert!(auction.submit_bid("bidder1", 100.0).unwrap());
        assert_eq!(auction.current_bid().unwrap().amount, 100.0);

        // Higher bid
        assert!(auction.submit_bid("bidder2", 115.0).unwrap());
        assert_eq!(auction.current_bid().unwrap().bidder, "bidder2");

        // Low bid rejected
        assert!(!auction.submit_bid("bidder3", 110.0).unwrap());
    }

    #[test]
    fn test_auction_close() {
        let mut auction = EnglishAuctionProtocol::new_as_auctioneer(100.0, 10.0);
        auction.state = EnglishAuctionState::Announced;

        auction.submit_bid("bidder1", 150.0).unwrap();
        let result = auction.close_auction().unwrap();

        assert!(result.is_some());
        assert_eq!(auction.winner(), Some("bidder1"));
    }
}
