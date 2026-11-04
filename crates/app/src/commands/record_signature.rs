//! Command: record_signature, from a typed `sign <hash8>` or a sign-card outcome;
//! on Bound, issues the receipt.

use proxenos_domain::deal::{Deal, DealState};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::event::DealEvent;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::party::{ParticipantId, Role};
use proxenos_domain::signature::MessageId;
use proxenos_domain::timestamp::Timestamp;
use serde::Serialize;

use crate::error::AppError;
use crate::ports::{CardAnswer, CardOutcome, Clock, DealRepository, ReceiptSigner};
use crate::receipt::{Receipt, issue};
use crate::stored::{SignRequestState, StoredDeal, load_deal};

pub enum RecordSignature {
    /// `hash` is at least the first 8 hex characters of the agreed terms hash.
    Typed {
        deal: DealId,
        signer: ParticipantId,
        hash: String,
        message_id: MessageId,
    },
    /// The card must be an open request this notary issued; its id is the message id.
    Card { deal: DealId, outcome: CardOutcome },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SignOutcome {
    Signed {
        role: Role,
    },
    Bound {
        role: Role,
        receipt: Box<Receipt>,
    },
    /// The side's owner declined the card; nothing was recorded.
    Declined {
        role: Role,
    },
    /// Someone else answered the card; it is closed and nothing was recorded.
    WrongParty {
        role: Role,
    },
    CardExpired {
        role: Role,
    },
}

pub async fn record_signature(
    repo: &impl DealRepository,
    clock: &impl Clock,
    signer: &impl ReceiptSigner,
    cmd: RecordSignature,
) -> Result<SignOutcome, AppError> {
    let now = clock.now();
    match cmd {
        RecordSignature::Typed {
            deal,
            signer: who,
            hash,
            message_id,
        } => {
            let mut stored = load_deal(repo, &deal).await?;
            let role = stored
                .deal
                .owner_role(&who)
                .ok_or(DomainError::NotBoundOwner)?;
            let hash = agreed_hash(&stored.deal, &hash)?;
            let act = Act {
                role,
                hash,
                message_id,
                who,
            };
            sign(repo, signer, &mut stored, act, now).await
        }
        RecordSignature::Card { deal, outcome } => {
            answer_card(repo, signer, &deal, outcome, now).await
        }
    }
}

/// Closes the open request the card answers; a sign by the side's bound owner signs the deal.
async fn answer_card(
    repo: &impl DealRepository,
    signer: &impl ReceiptSigner,
    deal: &DealId,
    outcome: CardOutcome,
    now: Timestamp,
) -> Result<SignOutcome, AppError> {
    let mut stored = load_deal(repo, deal).await?;
    let i = stored
        .sign_requests
        .iter()
        .position(|r| r.request_id == outcome.request && r.state == SignRequestState::Open)
        .ok_or(AppError::UnknownSignRequest)?;
    let request = stored.sign_requests[i].clone();
    let role = request.role;
    let owner = stored.deal.binding(role).map(|b| b.owner.clone());
    let (state, answered_by, result) = match outcome.answer {
        CardAnswer::Expired => (
            SignRequestState::Expired,
            None,
            SignOutcome::CardExpired { role },
        ),
        CardAnswer::Sign(p) | CardAnswer::Decline(p) if Some(&p) != owner.as_ref() => (
            SignRequestState::WrongParty,
            Some(p),
            SignOutcome::WrongParty { role },
        ),
        CardAnswer::Decline(p) => (
            SignRequestState::Declined,
            Some(p),
            SignOutcome::Declined { role },
        ),
        CardAnswer::Sign(p) => {
            let message_id = MessageId::new(request.request_id.0.clone())?;
            let hash = request.terms_hash;
            let req = &mut stored.sign_requests[i];
            req.state = SignRequestState::Signed;
            req.answered_by = Some(p.clone());
            let act = Act {
                role,
                hash,
                message_id,
                who: p,
            };
            return sign(repo, signer, &mut stored, act, now).await;
        }
    };
    let req = &mut stored.sign_requests[i];
    req.state = state;
    req.answered_by = answered_by;
    repo.save(&stored, &[], now).await?;
    Ok(result)
}

/// One signing act, already resolved to a side and a full hash.
struct Act {
    role: Role,
    hash: TermsHash,
    message_id: MessageId,
    who: ParticipantId,
}

async fn sign(
    repo: &impl DealRepository,
    signer: &impl ReceiptSigner,
    stored: &mut StoredDeal,
    act: Act,
    now: Timestamp,
) -> Result<SignOutcome, AppError> {
    let Act {
        role,
        hash,
        message_id,
        who,
    } = act;
    let events = stored
        .deal
        .record_signature(role, hash, message_id, who, now)?;
    let bound = events.iter().any(|e| matches!(e, DealEvent::Bound { .. }));
    if bound {
        stored.receipt = Some(
            issue(&stored.deal, now, signer).ok_or(AppError::Internal("bound without receipt"))?,
        );
    }
    repo.save(stored, &events, now).await?;
    Ok(match (&stored.receipt, bound) {
        (Some(receipt), true) => SignOutcome::Bound {
            role,
            receipt: Box::new(receipt.clone()),
        },
        _ => SignOutcome::Signed { role },
    })
}

/// Resolves a hash prefix against the current agreement.
fn agreed_hash(deal: &Deal, prefix: &str) -> Result<TermsHash, DomainError> {
    match deal.state() {
        DealState::Agreed(h) | DealState::PartlySigned(h) => {
            let prefix = prefix.trim().to_ascii_lowercase();
            if prefix.len() >= 8 && h.to_string().starts_with(&prefix) {
                Ok(h)
            } else {
                Err(DomainError::HashMismatch)
            }
        }
        s if s.is_terminal() => Err(DomainError::DealClosed),
        _ => Err(DomainError::NothingToSign),
    }
}
