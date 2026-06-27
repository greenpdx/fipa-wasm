//! FIPA ACL performatives (the standard set, FIPA00037).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Performative {
    AcceptProposal,
    Agree,
    Cancel,
    Cfp,
    Confirm,
    Disconfirm,
    Failure,
    Inform,
    InformIf,
    InformRef,
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

#[derive(Debug, Error)]
#[error("unknown performative: {0}")]
pub struct UnknownPerformative(pub String);

impl Performative {
    pub const ALL: [Performative; 22] = {
        use Performative::*;
        [
            AcceptProposal, Agree, Cancel, Cfp, Confirm, Disconfirm, Failure, Inform, InformIf,
            InformRef, NotUnderstood, Propagate, Propose, Proxy, QueryIf, QueryRef, Refuse,
            RejectProposal, Request, RequestWhen, RequestWhenever, Subscribe,
        ]
    };

    /// The lowercase, hyphenated FIPA name, e.g. `accept-proposal`.
    pub const fn as_str(self) -> &'static str {
        use Performative::*;
        match self {
            AcceptProposal => "accept-proposal",
            Agree => "agree",
            Cancel => "cancel",
            Cfp => "cfp",
            Confirm => "confirm",
            Disconfirm => "disconfirm",
            Failure => "failure",
            Inform => "inform",
            InformIf => "inform-if",
            InformRef => "inform-ref",
            NotUnderstood => "not-understood",
            Propagate => "propagate",
            Propose => "propose",
            Proxy => "proxy",
            QueryIf => "query-if",
            QueryRef => "query-ref",
            Refuse => "refuse",
            RejectProposal => "reject-proposal",
            Request => "request",
            RequestWhen => "request-when",
            RequestWhenever => "request-whenever",
            Subscribe => "subscribe",
        }
    }
}

impl std::str::FromStr for Performative {
    type Err = UnknownPerformative;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Performative::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| UnknownPerformative(s.to_string()))
    }
}

impl std::fmt::Display for Performative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
