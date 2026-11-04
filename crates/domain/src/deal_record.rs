//! DealRecord: the stored form of a Deal; loading it re-checks every invariant.

use std::collections::HashSet;

use serde::Deserialize;

use super::{Deal, DealState};
use crate::deal_id::DealId;
use crate::error::DomainError;
use crate::hash::TermsHash;
use crate::kind::DealKind;
use crate::offer::{Offer, OfferId};
use crate::party::{PartyBinding, Role, RoomId};
use crate::signature::Signature;
use crate::timestamp::Timestamp;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DealRecord {
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

impl TryFrom<DealRecord> for Deal {
    type Error = DomainError;
    fn try_from(r: DealRecord) -> Result<Self, DomainError> {
        let deal = Deal {
            id: r.id,
            kind: r.kind,
            room: r.room,
            state: r.state,
            max_rounds: r.max_rounds,
            deadline: r.deadline,
            opener: r.opener,
            offers: r.offers,
            agreed: r.agreed,
            sealed: r.sealed,
            desks: r.desks,
            bindings: r.bindings,
            signatures: r.signatures,
        };
        deal.validate()?;
        Ok(deal)
    }
}

fn ensure(ok: bool, what: &'static str) -> Result<(), DomainError> {
    if ok {
        Ok(())
    } else {
        Err(DomainError::InvalidRecord(what))
    }
}

impl Deal {
    fn validate(&self) -> Result<(), DomainError> {
        ensure(self.max_rounds > 0, "max_rounds")?;
        self.validate_parties()?;
        self.validate_offers()?;
        let hash = self.validate_agreement()?;
        self.validate_signatures(hash)
    }

    fn validate_parties(&self) -> Result<(), DomainError> {
        for role in [Role::A, Role::B] {
            if let Some(b) = self.binding(role) {
                ensure(self.desk(role) == Some(&b.desk), "binding desk")?;
            }
        }
        let owners: Vec<_> = self.bindings.iter().flatten().map(|b| &b.owner).collect();
        let agents: Vec<_> = self
            .bindings
            .iter()
            .flatten()
            .filter_map(|b| b.agent.as_ref())
            .collect();
        let all: HashSet<_> = owners.iter().chain(&agents).collect();
        ensure(
            all.len() == owners.len() + agents.len(),
            "participants not distinct",
        )
    }

    fn validate_offers(&self) -> Result<(), DomainError> {
        let both_sealed = self.sealed.iter().all(|s| *s);
        let sealed_ok = match self.state {
            DealState::AwaitingSeals => !both_sealed,
            DealState::Withdrawn | DealState::Expired => true,
            _ => both_sealed,
        };
        ensure(sealed_ok, "seals")?;
        ensure(self.offers.is_empty() || both_sealed, "offers before seals")?;
        ensure(
            self.offers.len() <= usize::from(self.max_rounds) * 2,
            "round cap",
        )?;
        for (i, o) in self.offers.iter().enumerate() {
            let by = if i % 2 == 0 {
                self.opener
            } else {
                self.opener.other()
            };
            ensure(o.id.number() as usize == i + 1, "offer id")?;
            ensure(o.by == by, "offer turn")?;
            ensure(usize::from(o.round) == i / 2 + 1, "offer round")?;
            ensure(o.terms.kind() == self.kind, "offer kind")?;
            let agent = self.binding(by).and_then(|b| b.agent.as_ref());
            ensure(agent.is_some(), "offer without negotiator")?;
        }
        let ids: HashSet<_> = self
            .offers
            .iter()
            .map(|o| &o.message_id)
            .chain(self.signatures.iter().map(|s| &s.message_id))
            .collect();
        ensure(
            ids.len() == self.offers.len() + self.signatures.len(),
            "message ids",
        )
    }

    /// Returns the recomputed agreed hash, if any.
    fn validate_agreement(&self) -> Result<Option<TermsHash>, DomainError> {
        let hash = match self.agreed {
            None => None,
            Some(id) => {
                let latest = self.latest_offer().filter(|o| o.id == id);
                let latest = latest.ok_or(DomainError::InvalidRecord("agreed offer"))?;
                let hash = self.hash_terms(&latest.terms);
                Some(hash.map_err(|_| DomainError::InvalidRecord("agreed without owners"))?)
            }
        };
        let ok = match self.state {
            DealState::AwaitingSeals | DealState::Open => hash.is_none(),
            DealState::Agreed(h) | DealState::PartlySigned(h) | DealState::Bound(h) => {
                hash == Some(h)
            }
            DealState::Withdrawn | DealState::Expired => true,
        };
        ensure(ok, "agreed hash")?;
        Ok(hash)
    }

    fn validate_signatures(&self, hash: Option<TermsHash>) -> Result<(), DomainError> {
        let count_ok = match self.state {
            DealState::PartlySigned(_) => self.signatures.len() == 1,
            DealState::Bound(_) => self.signatures.len() == 2,
            DealState::Withdrawn | DealState::Expired => self.signatures.len() < 2,
            _ => self.signatures.is_empty(),
        };
        ensure(count_ok, "signature count")?;
        for s in &self.signatures {
            ensure(Some(s.terms_hash) == hash, "signature hash")?;
            ensure(
                self.binding(s.role).map(|b| &b.owner) == Some(&s.signer),
                "signer",
            )?;
            ensure(s.at < self.deadline, "signature time")?;
        }
        let roles: HashSet<_> = self.signatures.iter().map(|s| s.role).collect();
        ensure(roles.len() == self.signatures.len(), "signature roles")
    }
}
