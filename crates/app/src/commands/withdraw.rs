//! Command: withdraw, by a bound owner.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::party::{ParticipantId, Role};

use crate::error::AppError;
use crate::ports::{Clock, DealRepository};
use crate::stored::load_deal;

pub struct Withdraw {
    pub deal: DealId,
    pub sender: ParticipantId,
}

/// Returns the withdrawing side.
pub async fn withdraw(
    repo: &impl DealRepository,
    clock: &impl Clock,
    cmd: Withdraw,
) -> Result<Role, AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let events = stored.deal.withdraw(&cmd.sender)?;
    repo.save(&stored, &events, clock.now()).await?;
    match events.as_slice() {
        [DealEvent::Withdrawn { by }] => Ok(*by),
        other => unreachable!("withdraw yields one Withdrawn event, got {other:?}"),
    }
}
