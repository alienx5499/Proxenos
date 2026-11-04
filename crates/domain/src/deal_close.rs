//! Deal closing commands: signatures, withdrawal, expiry.

use super::{Deal, DealState};
use crate::error::DomainError;
use crate::event::{DealEvent, ExpiryReason};
use crate::hash::TermsHash;
use crate::party::{ParticipantId, Role};
use crate::signature::{MessageId, Signature};
use crate::timestamp::Timestamp;

impl Deal {
    /// Records `signer` signing for `role` at `now` (the notary's clock): only for the
    /// agreed hash, from that role's bound owner, once per side, with an unused message id.
    pub fn record_signature(
        &mut self,
        role: Role,
        terms_hash: TermsHash,
        message_id: MessageId,
        signer: ParticipantId,
        now: Timestamp,
    ) -> Result<Vec<DealEvent>, DomainError> {
        let hash = match self.state {
            DealState::Agreed(h) | DealState::PartlySigned(h) => h,
            DealState::AwaitingSeals | DealState::Open => return Err(DomainError::NothingToSign),
            _ => return Err(DomainError::DealClosed),
        };
        if now >= self.deadline {
            return Err(DomainError::DeadlinePassed);
        }
        if terms_hash != hash {
            return Err(DomainError::HashMismatch);
        }
        if self.binding(role).map(|b| &b.owner) != Some(&signer) {
            return Err(DomainError::NotBoundOwner);
        }
        if self.signatures.iter().any(|s| s.role == role) {
            return Err(DomainError::AlreadySigned);
        }
        if self.message_used(&message_id) {
            return Err(DomainError::DuplicateMessage);
        }
        let signature = Signature {
            role,
            terms_hash,
            message_id,
            signer,
            at: now,
        };
        self.signatures.push(signature.clone());
        let mut events = vec![DealEvent::Signed { signature }];
        if self.signatures.len() == 2 {
            self.state = DealState::Bound(hash);
            events.push(DealEvent::Bound { hash });
        } else {
            self.state = DealState::PartlySigned(hash);
        }
        Ok(events)
    }

    /// A bound owner may withdraw their side at any point before the deal is bound.
    pub fn withdraw(&mut self, actor: &ParticipantId) -> Result<Vec<DealEvent>, DomainError> {
        self.ensure_not_closed()?;
        let by = self.owner_role(actor).ok_or(DomainError::NotBoundOwner)?;
        self.state = DealState::Withdrawn;
        Ok(vec![DealEvent::Withdrawn { by }])
    }

    /// Expires a deal whose deadline has passed; no events when nothing is due.
    pub fn expire_due(&mut self, now: Timestamp) -> Vec<DealEvent> {
        if self.state.is_terminal() || now < self.deadline {
            return Vec::new();
        }
        self.state = DealState::Expired;
        vec![DealEvent::Expired {
            reason: ExpiryReason::Deadline,
        }]
    }
}
