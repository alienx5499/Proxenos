//! Command: submit_offer, checked against the offerer's own sealed limits.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::event::{DealEvent, ExpiryReason};
use proxenos_domain::kind::Field;
use proxenos_domain::offer::{Note, Offer};
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::signature::MessageId;
use proxenos_domain::terms::Terms;
use serde::Serialize;

use super::DeskOnly;
use super::{ensure_move_room, in_own_desk, negotiator_role, own_limits};
use crate::error::AppError;
use crate::ports::{Clock, DealRepository, LimitVaultPort};
use crate::stored::{Check, CheckKind, load_deal};

#[derive(Clone)]
pub struct SubmitOffer {
    pub deal: DealId,
    pub sender: ParticipantId,
    pub room: RoomId,
    pub terms: Terms,
    pub note: Note,
    pub message_id: MessageId,
}

/// Failing fields only exist on the desk-room variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum OfferOutcome {
    Made {
        offer: Offer,
    },
    RefusedInDesk {
        failing_fields: Vec<Field>,
    },
    /// The failing fields go to the mover's desk only; the deal room sees the refusal.
    RefusedInDealRoom {
        #[serde(skip)]
        desk_detail: DeskOnly<Vec<Field>>,
    },
    /// The round cap was reached; the deal expired instead.
    Expired,
}

pub async fn submit_offer(
    repo: &impl DealRepository,
    vault: &impl LimitVaultPort,
    clock: &impl Clock,
    cmd: SubmitOffer,
) -> Result<OfferOutcome, AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let role = negotiator_role(&stored.deal, &cmd.sender)?;
    ensure_move_room(&stored.deal, role, &cmd.room)?;
    let desk = in_own_desk(&stored.deal, role, &cmd.room);
    let own = own_limits(vault, &stored, role)?;
    let now = clock.now();
    let round = stored.deal.round();
    let result =
        stored
            .deal
            .submit_offer(&cmd.sender, cmd.terms, cmd.note, cmd.message_id, &own, now);
    let (outcome, events, failing) = offer_outcome(result, desk, role)?;
    if let Some(failing_fields) = failing {
        stored.party_mut(role).record_check(Check {
            kind: CheckKind::Offer,
            round,
            within: failing_fields.is_empty(),
            failing_fields,
            at: now,
        });
    }
    repo.save(&stored, &events, now).await?;
    Ok(outcome)
}

/// The outcome, the events to save, and the failing fields of the own-limit check (none when
/// the offer was never checked because the deal expired).
type Classified = (OfferOutcome, Vec<DealEvent>, Option<Vec<Field>>);

fn offer_outcome(
    result: Result<Vec<DealEvent>, DomainError>,
    desk: bool,
    role: Role,
) -> Result<Classified, AppError> {
    Ok(match result {
        Ok(events) => match events.as_slice() {
            [DealEvent::OfferMade { offer }] => {
                let outcome = OfferOutcome::Made {
                    offer: offer.clone(),
                };
                (outcome, events, Some(Vec::new()))
            }
            [
                DealEvent::Expired {
                    reason: ExpiryReason::RoundCap,
                },
            ] => (OfferOutcome::Expired, events, None),
            _ => return Err(AppError::Internal("unexpected events from submit_offer")),
        },
        Err(DomainError::OutsideOwnLimits(fields)) => {
            let outcome = if desk {
                OfferOutcome::RefusedInDesk {
                    failing_fields: fields.clone(),
                }
            } else {
                OfferOutcome::RefusedInDealRoom {
                    desk_detail: DeskOnly(fields.clone()),
                }
            };
            (
                outcome,
                vec![DealEvent::OfferRefused { role }],
                Some(fields),
            )
        }
        Err(e) => return Err(e.into()),
    })
}
