//! Command: accept_offer, checked against the acceptor's own sealed limits.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::event::DealEvent;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::kind::Field;
use proxenos_domain::offer::OfferId;
use proxenos_domain::party::{ParticipantId, RoomId};
use serde::Serialize;

use super::DeskOnly;
use super::{ensure_move_room, in_own_desk, negotiator_role, own_limits};
use crate::error::AppError;
use crate::ports::{Clock, DealRepository, LimitVaultPort};
use crate::stored::{Check, CheckKind, load_deal};

pub struct AcceptOffer {
    pub deal: DealId,
    pub sender: ParticipantId,
    pub room: RoomId,
    pub offer_id: OfferId,
}

/// Failing fields only exist on the desk-room variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AcceptOutcome {
    Agreed {
        hash: TermsHash,
    },
    RefusedInDesk {
        failing_fields: Vec<Field>,
    },
    /// The failing fields go to the mover's desk only; the deal room sees the refusal.
    RefusedInDealRoom {
        #[serde(skip)]
        desk_detail: DeskOnly<Vec<Field>>,
    },
}

pub async fn accept_offer(
    repo: &impl DealRepository,
    vault: &impl LimitVaultPort,
    clock: &impl Clock,
    cmd: AcceptOffer,
) -> Result<AcceptOutcome, AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let role = negotiator_role(&stored.deal, &cmd.sender)?;
    ensure_move_room(&stored.deal, role, &cmd.room)?;
    let desk = in_own_desk(&stored.deal, role, &cmd.room);
    let own = own_limits(vault, &stored, role)?;
    let now = clock.now();
    let round = stored.deal.round();
    let (outcome, events, failing_fields) =
        match stored.deal.accept(&cmd.sender, cmd.offer_id, &own, now) {
            Ok(events) => match events.as_slice() {
                [DealEvent::Accepted { hash, .. }] => {
                    (AcceptOutcome::Agreed { hash: *hash }, events, Vec::new())
                }
                _ => return Err(AppError::Internal("unexpected events from accept")),
            },
            Err(DomainError::OutsideOwnLimits(fields)) => {
                let outcome = if desk {
                    AcceptOutcome::RefusedInDesk {
                        failing_fields: fields.clone(),
                    }
                } else {
                    AcceptOutcome::RefusedInDealRoom {
                        desk_detail: DeskOnly(fields.clone()),
                    }
                };
                (outcome, vec![DealEvent::AcceptRefused { role }], fields)
            }
            Err(e) => return Err(e.into()),
        };
    stored.party_mut(role).record_check(Check {
        kind: CheckKind::Accept,
        round,
        within: failing_fields.is_empty(),
        failing_fields,
        at: now,
    });
    repo.save(&stored, &events, now).await?;
    Ok(outcome)
}
