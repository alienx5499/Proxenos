//! Commands: open_deal (deal-room kickoff) and record_desk (desk kickoff).

use proxenos_domain::deal::Deal;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use serde::Serialize;

use crate::error::AppError;
use crate::ports::{Clock, DealReader, DealRepository, RepoError, SecretSource};
use crate::secret::seal_token;
use crate::stored::{PendingBinding, StoredDeal, load_deal};

pub struct OpenDeal {
    pub id: DealId,
    pub kind: DealKind,
    pub room: RoomId,
    pub deadline: Timestamp,
    pub max_rounds: u16,
    /// Who makes the first offer; kickoffs use `DEFAULT_OPENER`.
    pub opener: Role,
    /// Owner and negotiator per side (index 0 is Role A), bound when that side seals.
    pub parties: Option<[PendingBinding; 2]>,
}

/// Plain seal tokens, returned once; only their keyed hashes are stored.
#[derive(Serialize)]
pub struct SealTokens {
    pub role_a: String,
    pub role_b: String,
}

pub async fn open_deal(
    repo: &impl DealRepository,
    clock: &impl Clock,
    secrets: &impl SecretSource,
    cmd: OpenDeal,
) -> Result<SealTokens, AppError> {
    open_deal_in(repo, clock, secrets, cmd, None).await
}

/// Like `open_deal`, also recording both desk rooms (index 0 is Role A) in the same save.
/// Refused when the deal room or a desk is already used by a live deal.
pub async fn open_deal_in(
    repo: &impl DealRepository,
    clock: &impl Clock,
    secrets: &impl SecretSource,
    cmd: OpenDeal,
    desks: Option<[RoomId; 2]>,
) -> Result<SealTokens, AppError> {
    let exists = || AppError::DealExists(cmd.id.clone());
    if repo.load(&cmd.id).await?.is_some() {
        return Err(exists());
    }
    ensure_unused(repo, &cmd.room, None).await?;
    for desk in desks.iter().flatten() {
        ensure_unused(repo, desk, None).await?;
    }
    let (deal, mut events) = Deal::open(
        cmd.id.clone(),
        cmd.kind,
        cmd.room.clone(),
        cmd.deadline,
        cmd.max_rounds,
        cmd.opener,
    )?;
    let tokens = SealTokens {
        role_a: seal_token(secrets.random()),
        role_b: seal_token(secrets.random()),
    };
    let mut stored = StoredDeal::new(deal);
    if let Some([a, b]) = desks {
        if a == b || &a == stored.deal.room() || &b == stored.deal.room() {
            return Err(DomainError::WrongRoom.into());
        }
        events.extend(stored.deal.record_desk(Role::A, a)?);
        events.extend(stored.deal.record_desk(Role::B, b)?);
    }
    stored.party_mut(Role::A).seal_token = Some(secrets.hash(&tokens.role_a));
    stored.party_mut(Role::B).seal_token = Some(secrets.hash(&tokens.role_b));
    if let Some([a, b]) = cmd.parties {
        stored.party_mut(Role::A).pending_binding = Some(a);
        stored.party_mut(Role::B).pending_binding = Some(b);
    }
    match repo.save(&stored, &events, clock.now()).await {
        Err(RepoError::Conflict) => Err(exists()),
        other => other.map(|()| tokens).map_err(Into::into),
    }
}

pub struct RecordDesk {
    pub deal: DealId,
    pub role: Role,
    pub room: RoomId,
}

/// A desk room must differ from the deal room and from the other side's desk.
pub async fn record_desk(
    repo: &impl DealRepository,
    clock: &impl Clock,
    cmd: RecordDesk,
) -> Result<(), AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let d = &stored.deal;
    if &cmd.room == d.room() || d.desk(cmd.role.other()) == Some(&cmd.room) {
        return Err(DomainError::WrongRoom.into());
    }
    ensure_unused(repo, &cmd.room, Some(&cmd.deal)).await?;
    let events = stored.deal.record_desk(cmd.role, cmd.room)?;
    repo.save(&stored, &events, clock.now()).await?;
    Ok(())
}

/// `RoomInUse` when a live deal other than `except` uses `room` as deal room or desk.
async fn ensure_unused(
    repo: &impl DealReader,
    room: &RoomId,
    except: Option<&DealId>,
) -> Result<(), AppError> {
    let live = repo
        .in_room(room)
        .await?
        .into_iter()
        .find(|s| !s.deal.state().is_terminal() && Some(s.deal.id()) != except);
    match live {
        Some(s) => Err(AppError::RoomInUse(s.deal.id().clone())),
        None => Ok(()),
    }
}
