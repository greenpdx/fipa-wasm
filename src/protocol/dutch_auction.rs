// protocol/dutch_auction.rs - FIPA Dutch Auction Protocol
//
//! FIPA Dutch Auction Protocol Implementation
//!
//! The Dutch Auction is a descending-price auction where:
//! - Auctioneer starts with a high price
//! - Price decreases at regular intervals
//! - First bidder to accept wins at the current price
//!
//! # Protocol Flow
//!
//! ```text
//! Auctioneer                    Bidders
//!     |                            |
//!     |--------- CFP ------------->|  (item at price X)
//!     |                            |
//!     |  ... no response ...       |
//!     |                            |
//!     |--------- CFP ------------->|  (item at price X-delta)
//!     |                            |
//!     |<-------- PROPOSE ----------|  (accept at current price)
//!     |                            |
//!     |--------- ACCEPT-PROPOSAL ->|  (sold to bidder)
//!     |                            |
//! ```

use super::state_machine::*;
use crate::proto;
use serde::{Deserialize, Serialize};

/// Price update in Dutch auction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceUpdate {
    /// Current price
    pub price: f64,
    /// Update number
    pub round: u32,
    /// Timestamp
    pub timestamp: i64,
}

/// Dutch Auction Protocol States
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DutchAuctionState {
    /// Initial state
    NotStarted,
    /// Auction announced, descending
    Descending,
    /// Bid received, processing
    BidReceived,
    /// Auction completed (sold)
    Sold,
    /// Auction failed (no buyer at reserve)
    Unsold,
    /// Auction was cancelled
    Cancelled,
}

impl DutchAuctionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DutchAuctionState::NotStarted => "not_started",
            DutchAuctionState::Descending => "descending",
            DutchAuctionState::BidReceived => "bid_received",
            DutchAuctionState::Sold => "sold",
            DutchAuctionState::Unsold => "unsold",
            DutchAuctionState::Cancelled => "cancelled",
        }
    }
}

/// Dutch Auction Protocol Implementation
#[derive(Debug)]
pub struct DutchAuctionProtocol {
    /// Current state
    state: DutchAuctionState,

    /// Conversation base
    base: ConversationBase,

    /// Item being auctioned
    item_description: Option<Vec<u8>>,

    /// Starting (maximum) price
    starting_price: f64,

    /// Reserve (minimum) price
    reserve_price: f64,

    /// Current price
    current_price: f64,

    /// Price decrement per round
    price_decrement: f64,

    /// Current round
    current_round: u32,

    /// Price history
    price_history: Vec<PriceUpdate>,

    /// Winning bidder
    winner: Option<String>,

    /// Final sale price
    sale_price: Option<f64>,
}

impl DutchAuctionProtocol {
    /// Create a new Dutch auction (as auctioneer)
    pub fn new_as_auctioneer(starting_price: f64, reserve_price: f64, price_decrement: f64) -> Self {
        Self {
            state: DutchAuctionState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Initiator),
            item_description: None,
            starting_price,
            reserve_price,
            current_price: starting_price,
            price_decrement,
            current_round: 0,
            price_history: vec![],
            winner: None,
            sale_price: None,
        }
    }

    /// Create a new Dutch auction (as bidder)
    pub fn new_as_bidder() -> Self {
        Self {
            state: DutchAuctionState::NotStarted,
            base: ConversationBase::new(uuid::Uuid::new_v4().to_string(), Role::Participant),
            item_description: None,
            starting_price: 0.0,
            reserve_price: 0.0,
            current_price: 0.0,
            price_decrement: 0.0,
            current_round: 0,
            price_history: vec![],
            winner: None,
            sale_price: None,
        }
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

    /// Get current price
    pub fn current_price(&self) -> f64 {
        self.current_price
    }

    /// Get current round
    pub fn current_round(&self) -> u32 {
        self.current_round
    }

    /// Get winner
    pub fn winner(&self) -> Option<&str> {
        self.winner.as_deref()
    }

    /// Get sale price
    pub fn sale_price(&self) -> Option<f64> {
        self.sale_price
    }

    /// Start the auction
    pub fn start(&mut self) -> Result<(), ProtocolError> {
        if self.state != DutchAuctionState::NotStarted {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "start".to_string(),
            });
        }

        self.current_price = self.starting_price;
        self.current_round = 1;

        self.price_history.push(PriceUpdate {
            price: self.current_price,
            round: self.current_round,
            timestamp: chrono::Utc::now().timestamp_millis(),
        });

        self.state = DutchAuctionState::Descending;
        Ok(())
    }

    /// Decrease price for next round
    pub fn decrease_price(&mut self) -> Result<f64, ProtocolError> {
        if self.state != DutchAuctionState::Descending {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "decrease".to_string(),
            });
        }

        let new_price = self.current_price - self.price_decrement;

        if new_price < self.reserve_price {
            // Reached reserve without a bid
            self.state = DutchAuctionState::Unsold;
            return Err(ProtocolError::ValidationFailed("Reached reserve price without bids".into()));
        }

        self.current_price = new_price;
        self.current_round += 1;

        self.price_history.push(PriceUpdate {
            price: self.current_price,
            round: self.current_round,
            timestamp: chrono::Utc::now().timestamp_millis(),
        });

        Ok(self.current_price)
    }

    /// Accept a bid at current price
    pub fn accept_bid(&mut self, bidder: &str) -> Result<f64, ProtocolError> {
        if self.state != DutchAuctionState::Descending {
            return Err(ProtocolError::InvalidTransition {
                from: self.state.as_str().to_string(),
                to: "accept_bid".to_string(),
            });
        }

        self.winner = Some(bidder.to_string());
        self.sale_price = Some(self.current_price);
        self.state = DutchAuctionState::Sold;

        Ok(self.current_price)
    }

    /// Validate state transition based on performative
    fn validate_transition(&self, performative: proto::Performative) -> Result<DutchAuctionState, ProtocolError> {
        use proto::Performative::*;
        use DutchAuctionState::*;

        match (&self.state, performative) {
            // Auctioneer sends CFP with current price
            (NotStarted, Cfp) => Ok(Descending),
            (Descending, Cfp) => Ok(Descending), // Price decrease announcement
            // Bidder proposes to accept current price
            (Descending, Propose) => Ok(BidReceived),
            // Auctioneer accepts the bid
            (BidReceived, AcceptProposal) => Ok(Sold),
            // Auction complete notification
            (Descending, Inform) => Ok(Unsold), // No bids
            (Sold, Inform) => Ok(Sold), // Confirmation
            // Failure
            (_, Failure) => Ok(Unsold),
            // Cancel
            (_, Cancel) => Ok(Cancelled),
            (state, perf) => Err(ProtocolError::InvalidTransition {
                from: state.as_str().to_string(),
                to: format!("{:?}", perf),
            }),
        }
    }
}

impl ProtocolStateMachine for DutchAuctionProtocol {
    fn protocol_type(&self) -> proto::ProtocolType {
        proto::ProtocolType::ProtocolDutchAuction
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
            proto::Performative::Cfp if self.state == DutchAuctionState::NotStarted => {
                self.item_description = Some(msg.content.clone());
            }
            proto::Performative::Propose => {
                if let Some(sender) = &msg.sender {
                    self.winner = Some(sender.name.clone());
                }
            }
            _ => {}
        }

        self.state = new_state;

        match &self.state {
            DutchAuctionState::Sold => Ok(ProcessResult::Complete(CompletionData {
                result: self.sale_price.map(|p| format!("{}", p).into_bytes()),
                ..Default::default()
            })),
            DutchAuctionState::Unsold => Ok(ProcessResult::Failed("No buyer found".into())),
            DutchAuctionState::Cancelled => Ok(ProcessResult::Failed("Auction cancelled".into())),
            _ => Ok(ProcessResult::Continue),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(
            self.state,
            DutchAuctionState::Sold | DutchAuctionState::Unsold | DutchAuctionState::Cancelled
        )
    }

    fn is_failed(&self) -> bool {
        matches!(
            self.state,
            DutchAuctionState::Unsold | DutchAuctionState::Cancelled
        )
    }

    fn expected_performatives(&self) -> Vec<proto::Performative> {
        use proto::Performative::*;

        match &self.state {
            DutchAuctionState::NotStarted => vec![Cfp],
            DutchAuctionState::Descending => vec![Cfp, Propose, Inform, Cancel],
            DutchAuctionState::BidReceived => vec![AcceptProposal, Cancel],
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
    fn test_dutch_auction_basics() {
        let auction = DutchAuctionProtocol::new_as_auctioneer(1000.0, 100.0, 50.0);
        assert_eq!(auction.state, DutchAuctionState::NotStarted);
        assert_eq!(auction.current_price(), 1000.0);
    }

    #[test]
    fn test_price_descent() {
        let mut auction = DutchAuctionProtocol::new_as_auctioneer(1000.0, 100.0, 50.0);
        auction.start().unwrap();

        assert_eq!(auction.current_price(), 1000.0);

        auction.decrease_price().unwrap();
        assert_eq!(auction.current_price(), 950.0);

        auction.decrease_price().unwrap();
        assert_eq!(auction.current_price(), 900.0);
    }

    #[test]
    fn test_bid_acceptance() {
        let mut auction = DutchAuctionProtocol::new_as_auctioneer(1000.0, 100.0, 50.0);
        auction.start().unwrap();
        auction.decrease_price().unwrap();
        auction.decrease_price().unwrap();

        let sale_price = auction.accept_bid("bidder1").unwrap();
        assert_eq!(sale_price, 900.0);
        assert_eq!(auction.winner(), Some("bidder1"));
        assert!(auction.is_complete());
    }
}
