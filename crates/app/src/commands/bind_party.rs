//! Command: bind_party, the join-code sender becomes that side's owner.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::{ParticipantId, Role, RoomId};

use crate::error::AppError;
use crate::ports::{Clock, DealRepository, SecretSource};
use crate::secret::normalize_join_code;
use crate::stored::{JOIN_ATTEMPTS, StoredDeal};

#[derive(Clone)]
pub struct BindParty {
    pub join_code: String,
    pub sender: ParticipantId,
    pub room: RoomId,
}

/// The room picks the waiting side; a wrong code counts against it, and after
/// `JOIN_ATTEMPTS` wrong codes join is disabled there. The code is single-use.
pub async fn bind_party(
    repo: &impl DealRepository,
    clock: &impl Clock,
    secrets: &impl SecretSource,
    cmd: BindParty,
) -> Result<(DealId, Role), AppError> {
    let hash = secrets.hash(&normalize_join_code(&cmd.join_code));
    let waiting: Vec<(StoredDeal, Role)> = repo
        .in_room(&cmd.room)
        .await?
        .into_iter()
        .filter(|s| !s.deal.state().is_terminal())
        .filter_map(|s| {
            let role = [Role::A, Role::B]
                .into_iter()
                .find(|r| s.deal.desk(*r) == Some(&cmd.room) && s.party(*r).join_code.is_some())?;
            Some((s, role))
        })
        .collect();
    if waiting.is_empty() {
        return Err(AppError::UnknownJoinCode);
    }
    if waiting
        .iter()
        .all(|(s, r)| s.party(*r).failed_joins >= JOIN_ATTEMPTS)
    {
        return Err(AppError::JoinLocked);
    }
    let now = clock.now();
    let matched = waiting.iter().position(|(s, r)| {
        s.party(*r).join_code == Some(hash) && s.party(*r).failed_joins < JOIN_ATTEMPTS
    });
    let Some(i) = matched else {
        let mut attempts_left = 0;
        for (mut stored, role) in waiting {
            let party = stored.party_mut(role);
            party.failed_joins = party.failed_joins.saturating_add(1);
            attempts_left = attempts_left.max(JOIN_ATTEMPTS.saturating_sub(party.failed_joins));
            repo.save(&stored, &[], now).await?;
        }
        return Err(AppError::WrongJoinCode { attempts_left });
    };
    let (mut stored, role) = waiting.into_iter().nth(i).expect("index from position");
    let events = stored.deal.bind_party(role, cmd.sender, &cmd.room)?;
    stored.party_mut(role).join_code = None;
    repo.save(&stored, &events, now).await?;
    Ok((stored.deal.id().clone(), role))
}
