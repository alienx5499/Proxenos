//! Command: bind_negotiator, the side is the one whose desk room the sender spoke in.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::party::{ParticipantId, Role, RoomId};

use crate::error::AppError;
use crate::ports::{Clock, DealRepository};
use crate::stored::load_deal;

#[derive(Clone)]
pub struct BindNegotiator {
    pub deal: DealId,
    pub sender: ParticipantId,
    pub room: RoomId,
    /// Ids that may never negotiate: the notary itself and registered relay ids.
    pub excluded: Vec<ParticipantId>,
}

pub async fn bind_negotiator(
    repo: &impl DealRepository,
    clock: &impl Clock,
    cmd: BindNegotiator,
) -> Result<Role, AppError> {
    bind_negotiator_named(repo, clock, cmd, None).await
}

/// Also records the negotiator's chat name, in the same save, for addressing its wakes.
pub async fn bind_negotiator_named(
    repo: &impl DealRepository,
    clock: &impl Clock,
    cmd: BindNegotiator,
    name: Option<String>,
) -> Result<Role, AppError> {
    if cmd.excluded.contains(&cmd.sender) {
        return Err(AppError::ExcludedNegotiator);
    }
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let role = [Role::A, Role::B]
        .into_iter()
        .find(|r| stored.deal.desk(*r) == Some(&cmd.room))
        .ok_or(DomainError::WrongRoom)?;
    let events = stored.deal.bind_negotiator(role, cmd.sender, &cmd.room)?;
    stored.party_mut(role).negotiator_name = name;
    repo.save(&stored, &events, clock.now()).await?;
    Ok(role)
}
