//! DealEvent: append-only log entries; limit values are not representable.
//!
//! No variant holds a `LimitSet`, `Constraint`, `OwnVerdict` or failing field:
//! checks are logged as booleans or outcomes only.

use serde::{Deserialize, Serialize};

use crate::deal_id::DealId;
use crate::hash::TermsHash;
use crate::kind::DealKind;
use crate::offer::{Offer, OfferId};
use crate::party::{ParticipantId, Role, RoomId};
use crate::signature::Signature;
use crate::timestamp::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryReason {
    Deadline,
    RoundCap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DealEvent {
    Opened {
        id: DealId,
        kind: DealKind,
        room: RoomId,
        max_rounds: u16,
        deadline: Timestamp,
        opener: Role,
    },
    DeskRecorded {
        role: Role,
        desk: RoomId,
    },
    /// The second seal also moves the deal from AwaitingSeals to Open.
    LimitsSealed {
        role: Role,
    },
    PartyBound {
        role: Role,
        owner: ParticipantId,
    },
    NegotiatorBound {
        role: Role,
        agent: ParticipantId,
    },
    /// Recorded by the application layer; probes do not change deal state.
    Probed {
        role: Role,
        within: bool,
    },
    OfferMade {
        offer: Offer,
    },
    /// Recorded by the application layer when the aggregate returns `OutsideOwnLimits`.
    OfferRefused {
        role: Role,
    },
    /// Recorded by the application layer when the aggregate returns `OutsideOwnLimits`.
    AcceptRefused {
        role: Role,
    },
    Accepted {
        by: Role,
        offer_id: OfferId,
        hash: TermsHash,
    },
    Signed {
        signature: Signature,
    },
    Bound {
        hash: TermsHash,
    },
    Withdrawn {
        by: Role,
    },
    Expired {
        reason: ExpiryReason,
    },
}
