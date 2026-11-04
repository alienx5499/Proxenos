//! Command: open_sign_card, one Switch sign card for a side, recorded as an open sign request.

use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;

use crate::error::AppError;
use crate::ports::{Clock, DealRepository, RequestId, SignCard, SwitchPort};
use crate::queries::label;
use crate::stored::{SignRequest, SignRequestState, load_deal};

pub struct OpenSignCard {
    pub deal: DealId,
    pub role: Role,
}

/// Switch refuses card expiries further out than this.
const CARD_MAX_SECS: i64 = 86_400;

/// Any earlier open card for the same side is superseded.
pub async fn open_sign_card(
    repo: &impl DealRepository,
    switch: &impl SwitchPort,
    clock: &impl Clock,
    cmd: OpenSignCard,
) -> Result<RequestId, AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let d = &stored.deal;
    let hash = match d.state() {
        DealState::Agreed(h) | DealState::PartlySigned(h) => h,
        s if s.is_terminal() => return Err(DomainError::DealClosed.into()),
        _ => return Err(DomainError::NothingToSign.into()),
    };
    if d.signatures().iter().any(|s| s.role == cmd.role) {
        return Err(DomainError::AlreadySigned.into());
    }
    let now = clock.now();
    let card = SignCard {
        room: d.room().clone(),
        deal: d.id().clone(),
        role: cmd.role,
        label: label(d.kind(), cmd.role),
        terms_hash: hash,
        round: d.round(),
        expires: d
            .deadline()
            .min(Timestamp(now.0.saturating_add(CARD_MAX_SECS))),
        attempt: 1 + stored
            .sign_requests
            .iter()
            .filter(|r| r.role == cmd.role && r.terms_hash == hash)
            .count(),
    };
    let request_id = switch.request_signature(&card).await?;
    for r in &mut stored.sign_requests {
        if r.role == cmd.role && r.state == SignRequestState::Open {
            r.state = SignRequestState::Superseded;
        }
    }
    stored.sign_requests.push(SignRequest {
        request_id: request_id.clone(),
        role: cmd.role,
        terms_hash: hash,
        state: SignRequestState::Open,
        answered_by: None,
        at: now,
    });
    repo.save(&stored, &[], now).await?;
    Ok(request_id)
}
