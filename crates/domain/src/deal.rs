//! Deal aggregate: AwaitingSeals -> Open -> Agreed -> PartlySigned -> Bound, plus Withdrawn / Expired.
//!
//! Turns: the opener (chosen at open, `DEFAULT_OPENER` = Role A) makes the first offer,
//! then turns alternate. A round is one offer from each side, so offer `n` (from 1) falls
//! in round `(n + 1) / 2`. After the second mover's offer in the last round, the opener may
//! still accept it; an offer from the opener at that point expires the deal
//! (`ExpiryReason::RoundCap`) instead of being recorded.
//!
//! Own-limit checks take the caller's `&LimitSet`, so they are enforced here while limits
//! are never stored on the deal. Commands take the acting participant and derive the role
//! from the bindings, so only a bound negotiator moves and only a bound owner withdraws. Every command validates before it mutates: on `Err`
//! the deal is unchanged.

use serde::{Deserialize, Serialize};

use crate::deal_id::DealId;
use crate::error::DomainError;
use crate::event::DealEvent;
use crate::hash::TermsHash;
use crate::kind::DealKind;
use crate::offer::{Offer, OfferId};
use crate::party::{ParticipantId, PartyBinding, Role, RoomId};
use crate::signature::{MessageId, Signature};
use crate::terms::Terms;
use crate::timestamp::Timestamp;

#[path = "deal_close.rs"]
mod close;
#[path = "deal_negotiate.rs"]
mod negotiate;
#[path = "deal_record.rs"]
mod record;
#[path = "deal_setup.rs"]
mod setup;

pub const DEFAULT_MAX_ROUNDS: u16 = 8;
pub const DEFAULT_OPENER: Role = Role::A;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DealState {
    AwaitingSeals,
    Open,
    Agreed(TermsHash),
    PartlySigned(TermsHash),
    Bound(TermsHash),
    Withdrawn,
    Expired,
}

impl DealState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            DealState::Bound(_) | DealState::Withdrawn | DealState::Expired
        )
    }
}

/// Deserialization re-checks every invariant (see `deal_record.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "record::DealRecord")]
pub struct Deal {
    id: DealId,
    kind: DealKind,
    room: RoomId,
    state: DealState,
    max_rounds: u16,
    deadline: Timestamp,
    opener: Role,
    offers: Vec<Offer>,
    agreed: Option<OfferId>,
    sealed: [bool; 2],
    desks: [Option<RoomId>; 2],
    bindings: [Option<PartyBinding>; 2],
    signatures: Vec<Signature>,
}

impl Deal {
    pub fn open(
        id: DealId,
        kind: DealKind,
        room: RoomId,
        deadline: Timestamp,
        max_rounds: u16,
        opener: Role,
    ) -> Result<(Self, Vec<DealEvent>), DomainError> {
        if max_rounds == 0 {
            return Err(DomainError::InvalidMaxRounds);
        }
        let event = DealEvent::Opened {
            id: id.clone(),
            kind,
            room: room.clone(),
            max_rounds,
            deadline,
            opener,
        };
        let deal = Self {
            id,
            kind,
            room,
            state: DealState::AwaitingSeals,
            max_rounds,
            deadline,
            opener,
            offers: Vec::new(),
            agreed: None,
            sealed: [false; 2],
            desks: [None, None],
            bindings: [None, None],
            signatures: Vec::new(),
        };
        Ok((deal, vec![event]))
    }

    pub fn id(&self) -> &DealId {
        &self.id
    }

    pub fn kind(&self) -> DealKind {
        self.kind
    }

    pub fn room(&self) -> &RoomId {
        &self.room
    }

    pub fn state(&self) -> DealState {
        self.state
    }

    pub fn max_rounds(&self) -> u16 {
        self.max_rounds
    }

    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    /// The round the next offer falls in, capped at `max_rounds`.
    pub fn round(&self) -> u16 {
        let next = u16::try_from(self.offers.len() / 2 + 1).unwrap_or(u16::MAX);
        next.min(self.max_rounds)
    }

    pub fn opener(&self) -> Role {
        self.opener
    }

    pub fn turn(&self) -> Role {
        if self.offers.len().is_multiple_of(2) {
            self.opener
        } else {
            self.opener.other()
        }
    }

    pub fn offers(&self) -> &[Offer] {
        &self.offers
    }

    pub fn latest_offer(&self) -> Option<&Offer> {
        self.offers.last()
    }

    /// The frozen terms once agreed.
    pub fn agreed_offer(&self) -> Option<&Offer> {
        let id = self.agreed?;
        self.offers.iter().find(|o| o.id == id)
    }

    pub fn is_sealed(&self, role: Role) -> bool {
        self.sealed[role.index()]
    }

    pub fn desk(&self, role: Role) -> Option<&RoomId> {
        self.desks[role.index()].as_ref()
    }

    pub fn binding(&self, role: Role) -> Option<&PartyBinding> {
        self.bindings[role.index()].as_ref()
    }

    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    /// The role whose bound negotiator is `actor`.
    pub fn negotiator_role(&self, actor: &ParticipantId) -> Option<Role> {
        [Role::A, Role::B]
            .into_iter()
            .find(|r| self.binding(*r).and_then(|b| b.agent.as_ref()) == Some(actor))
    }

    /// The role whose bound owner is `actor`.
    pub fn owner_role(&self, actor: &ParticipantId) -> Option<Role> {
        [Role::A, Role::B]
            .into_iter()
            .find(|r| self.binding(*r).map(|b| &b.owner) == Some(actor))
    }

    /// The hash of `terms` for this deal; needs both owners bound.
    fn hash_terms(&self, terms: &Terms) -> Result<TermsHash, DomainError> {
        let owner = |r: Role| self.binding(r).map(|b| &b.owner);
        let (Some(a), Some(b)) = (owner(Role::A), owner(Role::B)) else {
            return Err(DomainError::PartyNotBound);
        };
        Ok(TermsHash::of(terms, &self.id, [a, b]))
    }

    fn message_used(&self, id: &MessageId) -> bool {
        self.offers.iter().any(|o| &o.message_id == id)
            || self.signatures.iter().any(|s| &s.message_id == id)
    }
}

#[cfg(test)]
#[path = "deal_fixtures.rs"]
pub(crate) mod fixtures;

#[cfg(test)]
#[path = "deal_tests.rs"]
mod tests;
