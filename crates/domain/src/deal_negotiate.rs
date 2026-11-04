//! Deal negotiation commands: offers and acceptance.

use super::{Deal, DealState};
use crate::error::DomainError;
use crate::event::{DealEvent, ExpiryReason};
use crate::limit::LimitSet;
use crate::offer::{Note, Offer, OfferId};
use crate::party::{ParticipantId, Role};
use crate::signature::MessageId;
use crate::terms::Terms;
use crate::timestamp::Timestamp;

impl Deal {
    /// `actor` must be a bound negotiator; its side is the offering role.
    pub fn submit_offer(
        &mut self,
        actor: &ParticipantId,
        terms: Terms,
        note: Note,
        message_id: MessageId,
        own: &LimitSet,
        now: Timestamp,
    ) -> Result<Vec<DealEvent>, DomainError> {
        let by = self.ensure_negotiator_move(actor, now)?;
        if by != self.turn() {
            return Err(DomainError::NotYourTurn);
        }
        if self.message_used(&message_id) {
            return Err(DomainError::DuplicateMessage);
        }
        if self.offers.len() >= usize::from(self.max_rounds) * 2 {
            self.state = DealState::Expired;
            let reason = ExpiryReason::RoundCap;
            return Ok(vec![DealEvent::Expired { reason }]);
        }
        if terms.kind() != self.kind {
            return Err(DomainError::KindMismatch);
        }
        self.check_own_limits(by, &terms, own)?;
        let number = u32::try_from(self.offers.len() + 1).map_err(|_| DomainError::Overflow)?;
        let offer = Offer {
            id: OfferId::new(number)?,
            by,
            round: self.round(),
            terms,
            note,
            message_id,
        };
        self.offers.push(offer.clone());
        Ok(vec![DealEvent::OfferMade { offer }])
    }

    /// Accepting the counterparty's latest offer freezes its terms and fixes the hash,
    /// which covers both bound owners, so both must be bound first.
    pub fn accept(
        &mut self,
        actor: &ParticipantId,
        offer_id: OfferId,
        own: &LimitSet,
        now: Timestamp,
    ) -> Result<Vec<DealEvent>, DomainError> {
        let by = self.ensure_negotiator_move(actor, now)?;
        let latest = self
            .latest_offer()
            .filter(|o| o.id == offer_id && o.by != by)
            .ok_or(DomainError::NotLatestOffer)?;
        self.check_own_limits(by, &latest.terms, own)?;
        let hash = self.hash_terms(&latest.terms)?;
        self.agreed = Some(offer_id);
        self.state = DealState::Agreed(hash);
        Ok(vec![DealEvent::Accepted { by, offer_id, hash }])
    }

    fn ensure_negotiator_move(
        &self,
        actor: &ParticipantId,
        now: Timestamp,
    ) -> Result<Role, DomainError> {
        match self.state {
            DealState::Open if now >= self.deadline => return Err(DomainError::DeadlinePassed),
            DealState::Open => {}
            DealState::AwaitingSeals => return Err(DomainError::AwaitingSeals),
            _ => return Err(DomainError::DealClosed),
        }
        self.negotiator_role(actor)
            .ok_or(DomainError::NotBoundNegotiator)
    }

    fn check_own_limits(&self, by: Role, terms: &Terms, own: &LimitSet) -> Result<(), DomainError> {
        if own.role() != by || own.kind() != self.kind {
            return Err(DomainError::WrongLimitSet);
        }
        let verdict = own.verdict(terms);
        if !verdict.within {
            return Err(DomainError::OutsideOwnLimits(verdict.failing_fields));
        }
        Ok(())
    }
}
