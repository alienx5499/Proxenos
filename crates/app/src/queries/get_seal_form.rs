//! Query: get_seal_form, which deal and side a seal token is for (no values).

use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;

use crate::error::AppError;
use crate::ports::{DealReader, SecretSource};
use crate::stored::load_deal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealForm {
    pub deal: DealId,
    pub kind: DealKind,
    pub role: Role,
}

pub async fn get_seal_form(
    repo: &impl DealReader,
    secrets: &impl SecretSource,
    token: &str,
) -> Result<SealForm, AppError> {
    let (deal, role) = repo
        .by_seal_token(&secrets.hash(token))
        .await?
        .ok_or(AppError::UnknownSealToken)?;
    let kind = load_deal(repo, &deal).await?.deal.kind();
    Ok(SealForm { deal, kind, role })
}
