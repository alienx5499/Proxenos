//! Command: seal_limits (seal token -> encrypted limits on the deal), returning the join code once.
//! A side named at open is bound (owner, then negotiator) in the same save.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::party::Role;
use serde::Serialize;

use crate::error::AppError;
use crate::ports::{Clock, DealReader, DealRepository, LimitVaultPort, SecretSource};
use crate::secret::{SecretHash, join_code, normalize_join_code};
use crate::stored::load_deal;

/// Fresh codes tried before giving up on a collision with a pending code.
const CODE_TRIES: usize = 5;

#[derive(Clone)]
pub struct SealLimits {
    pub token: String,
    pub limits: LimitSet,
}

#[derive(Debug, Serialize)]
pub struct Sealed {
    pub deal: DealId,
    pub role: Role,
    /// Plain join code (`XXXX-XXXX`), shown once on the seal page.
    pub join_code: String,
    /// Owner and negotiator were bound from the ids given at open; the code is then spent.
    pub bound: bool,
}

/// Limits, token use and join code are written in one save; nothing else changes on error.
pub async fn seal_limits(
    repo: &impl DealRepository,
    vault: &impl LimitVaultPort,
    clock: &impl Clock,
    secrets: &impl SecretSource,
    cmd: SealLimits,
) -> Result<Sealed, AppError> {
    let (id, role) = repo
        .by_seal_token(&secrets.hash(&cmd.token))
        .await?
        .ok_or(AppError::UnknownSealToken)?;
    if cmd.limits.role() != role {
        return Err(AppError::WrongSide);
    }
    let mut stored = load_deal(repo, &id).await?;
    let mut events = stored.deal.seal_limits(&cmd.limits)?;
    let pending = stored.party(role).pending_binding.clone();
    if let Some(p) = &pending {
        let desk = stored
            .deal
            .desk(role)
            .cloned()
            .ok_or(DomainError::DeskNotRecorded)?;
        events.extend(stored.deal.bind_party(role, p.owner.clone(), &desk)?);
        events.extend(stored.deal.bind_negotiator(role, p.agent.clone(), &desk)?);
    }
    let sealed = vault.encrypt(&id, role, &cmd.limits)?;
    let (code, hash) = unique_join_code(repo, secrets).await?;
    let party = stored.party_mut(role);
    party.seal_token = None;
    party.sealed = Some(sealed);
    party.join_code = pending.is_none().then_some(hash);
    party.pending_binding = None;
    repo.save(&stored, &events, clock.now()).await?;
    Ok(Sealed {
        deal: id,
        role,
        join_code: code,
        bound: pending.is_some(),
    })
}

async fn unique_join_code(
    repo: &impl DealReader,
    secrets: &impl SecretSource,
) -> Result<(String, SecretHash), AppError> {
    for _ in 0..CODE_TRIES {
        let code = join_code(secrets.random());
        let hash = secrets.hash(&normalize_join_code(&code));
        if repo.by_join_code(&hash).await?.is_none() {
            return Ok((code, hash));
        }
    }
    Err(AppError::Internal("no unused join code"))
}
