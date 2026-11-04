//! Command: admit_parties, one notary policy rule for the deal group, added once.

use proxenos_domain::deal_id::DealId;

use crate::error::AppError;
use crate::ports::{Clock, DealRepository, SwitchAdminPort};
use crate::stored::load_deal;

pub struct AdmitParties {
    pub deal: DealId,
    pub group_name: String,
    /// Gateway usernames of both owners.
    pub owner_usernames: Vec<String>,
    pub agent_ids: Vec<String>,
}

/// Returns the group id. When this deal was already admitted for that group, no new rule is added.
pub async fn admit_parties(
    repo: &impl DealRepository,
    admin: &impl SwitchAdminPort,
    clock: &impl Clock,
    cmd: AdmitParties,
) -> Result<String, AppError> {
    if cmd.owner_usernames.len() != 2 || cmd.agent_ids.len() != 2 {
        return Err(AppError::AdmitNeedsTwoEach);
    }
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let group = admin.group_id(&cmd.group_name).await?;
    if stored.group_id.as_ref() == Some(&group) {
        return Ok(group);
    }
    let mut users = Vec::with_capacity(2);
    for name in &cmd.owner_usernames {
        users.push(admin.external_user_id(name).await?);
    }
    admin.admit_parties(&group, &users, &cmd.agent_ids).await?;
    stored.group_id = Some(group.clone());
    repo.save(&stored, &[], clock.now()).await?;
    Ok(group)
}
