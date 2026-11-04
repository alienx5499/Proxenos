//! Command: expire_due, expires every active deal past its deadline.

use proxenos_domain::deal_id::DealId;

use crate::error::AppError;
use crate::ports::{Clock, DealRepository};
use crate::stored::load_deal;

#[derive(Debug, Default)]
pub struct ExpireReport {
    pub expired: Vec<DealId>,
    /// Deals that could not be checked or saved; the others still ran.
    pub failed: Vec<(DealId, AppError)>,
}

pub async fn expire_due(
    repo: &impl DealRepository,
    clock: &impl Clock,
) -> Result<ExpireReport, AppError> {
    let now = clock.now();
    let mut report = ExpireReport::default();
    for id in repo.active().await? {
        match expire_one(repo, &id, now).await {
            Ok(true) => report.expired.push(id),
            Ok(false) => {}
            Err(e) => report.failed.push((id, e)),
        }
    }
    Ok(report)
}

async fn expire_one(
    repo: &impl DealRepository,
    id: &DealId,
    now: proxenos_domain::timestamp::Timestamp,
) -> Result<bool, AppError> {
    let mut stored = load_deal(repo, id).await?;
    let events = stored.deal.expire_due(now);
    if events.is_empty() {
        return Ok(false);
    }
    repo.save(&stored, &events, now).await?;
    Ok(true)
}
