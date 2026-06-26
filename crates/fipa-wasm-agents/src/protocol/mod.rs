// protocol/mod.rs - FIPA Protocol Implementations

//! FIPA Protocol state machine implementations.
//!
//! This module provides type-safe implementations of FIPA interaction protocols:
//!
//! - `RequestProtocol` - Simple request-response pattern
//! - `QueryProtocol` - Information retrieval (query-if, query-ref)
//! - `ContractNetProtocol` - Task allocation through bidding
//! - `SubscribeProtocol` - Continuous notifications
//!
//! Each protocol is implemented as a state machine that validates
//! message sequences and manages transitions.
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::protocol::*;
//!
//! let mut protocol = RequestProtocol::new(Role::Initiator);
//!
//! // Send request
//! let request = create_request_message(receiver, content);
//! protocol.process(request)?;
//!
//! // Process response
//! let response = receive_message();
//! match protocol.process(response)? {
//!     ProcessResult::Continue => { /* wait for more */ }
//!     ProcessResult::Complete(data) => { /* done! */ }
//!     ProcessResult::Failed(err) => { /* handle error */ }
//! }
//! ```

mod brokering;
mod contract_net;
mod dutch_auction;
mod english_auction;
mod iterated_contract_net;
mod propose;
mod query;
mod recruiting;
mod request;
mod state_machine;
mod subscribe;

pub use brokering::{BrokeringProtocol, BrokeringState, ProviderInfo, ProviderStatus};
pub use contract_net::{ContractNetProtocol, ContractNetState, Proposal};
pub use dutch_auction::{DutchAuctionProtocol, DutchAuctionState, PriceUpdate};
pub use english_auction::{Bid, EnglishAuctionProtocol, EnglishAuctionState};
pub use iterated_contract_net::{IteratedContractNetProtocol, IteratedContractNetState, NegotiationRound};
pub use propose::{ProposeProtocol, ProposeState};
pub use query::{QueryProtocol, QueryState, QueryType};
pub use recruiting::{Candidate, RecruitingProtocol, RecruitingState};
pub use request::{RequestProtocol, RequestState};
pub use state_machine::{
    create_response, create_state_machine, CompletionData, ConversationBase, ProcessResult,
    ProtocolError, ProtocolStateMachine, Role,
};
pub use subscribe::{SubscribeProtocol, SubscribeState};
