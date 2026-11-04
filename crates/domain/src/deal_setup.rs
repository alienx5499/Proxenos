//! Deal setup commands: desk rooms, seals, party and negotiator binding.

use super::{Deal, DealState};
use crate::error::DomainError;
use crate::event::DealEvent;
use crate::limit::LimitSet;
use crate::party::{ParticipantId, PartyBinding, Role, RoomId};

impl Deal {
    pub fn record_desk(&mut self, role: Role, desk: RoomId) -> Result<Vec<DealEvent>, DomainError> {
        self.ensure_not_closed()?;
        let slot = &mut self.desks[role.index()];
        if slot.is_some() {
            return Err(DomainError::DeskAlreadyRecorded);
        }
        *slot = Some(desk.clone());
        Ok(vec![DealEvent::DeskRecorded { role, desk }])
    }

    /// The deal opens for offers once both sides have sealed.
    pub fn seal_limits(&mut self, limits: &LimitSet) -> Result<Vec<DealEvent>, DomainError> {
        if limits.kind() != self.kind {
            return Err(DomainError::WrongLimitSet);
        }
        let role = limits.role();
        if self.sealed[role.index()] {
            return Err(DomainError::AlreadySealed);
        }
        if self.state != DealState::AwaitingSeals {
            return Err(DomainError::DealClosed);
        }
        self.sealed[role.index()] = true;
        if self.sealed.iter().all(|s| *s) {
            self.state = DealState::Open;
        }
        Ok(vec![DealEvent::LimitsSealed { role }])
    }

    /// Binds the join-code sender as owner; only in the desk room recorded for that side.
    pub fn bind_party(
        &mut self,
        role: Role,
        owner: ParticipantId,
        room: &RoomId,
    ) -> Result<Vec<DealEvent>, DomainError> {
        self.ensure_not_closed()?;
        let desk = self.desks[role.index()]
            .clone()
            .ok_or(DomainError::DeskNotRecorded)?;
        if &desk != room {
            return Err(DomainError::WrongRoom);
        }
        if self.bindings[role.index()].is_some() {
            return Err(DomainError::AlreadyBound);
        }
        if self.owner_role(&owner).is_some() {
            return Err(DomainError::OwnerIsCounterparty);
        }
        if self.negotiator_role(&owner).is_some() {
            return Err(DomainError::OwnerIsNegotiator);
        }
        self.bindings[role.index()] = Some(PartyBinding {
            owner: owner.clone(),
            agent: None,
            desk,
        });
        Ok(vec![DealEvent::PartyBound { role, owner }])
    }

    pub fn bind_negotiator(
        &mut self,
        role: Role,
        agent: ParticipantId,
        room: &RoomId,
    ) -> Result<Vec<DealEvent>, DomainError> {
        if !matches!(self.state, DealState::AwaitingSeals | DealState::Open) {
            return Err(DomainError::DealClosed);
        }
        if self.owner_role(&agent).is_some() {
            return Err(DomainError::OwnerIsNegotiator);
        }
        if self.negotiator_role(&agent).is_some_and(|r| r != role) {
            return Err(DomainError::NegotiatorIsCounterparty);
        }
        let binding = self.bindings[role.index()]
            .as_mut()
            .ok_or(DomainError::PartyNotBound)?;
        if &binding.desk != room {
            return Err(DomainError::WrongRoom);
        }
        if binding.agent.is_some() {
            return Err(DomainError::NegotiatorAlreadyBound);
        }
        binding.agent = Some(agent.clone());
        Ok(vec![DealEvent::NegotiatorBound { role, agent }])
    }

    pub(super) fn ensure_not_closed(&self) -> Result<(), DomainError> {
        if self.state.is_terminal() {
            return Err(DomainError::DealClosed);
        }
        Ok(())
    }
}
