//! Commands: record_deal_sheet (the document written, sheet clean) and mark_sheet_dirty.

use proxenos_domain::deal_id::DealId;

use crate::error::AppError;
use crate::ports::{Clock, DealRepository, DocId};
use crate::stored::load_deal;

/// Records the deal-room document holding the sheet and clears the dirty flag.
pub async fn record_deal_sheet(
    repo: &impl DealRepository,
    clock: &impl Clock,
    deal: &DealId,
    doc: &DocId,
) -> Result<(), AppError> {
    let mut stored = load_deal(repo, deal).await?;
    if stored.deal_sheet_doc.as_deref() == Some(doc.0.as_str()) && !stored.sheet_dirty {
        return Ok(());
    }
    stored.deal_sheet_doc = Some(doc.0.clone());
    stored.sheet_dirty = false;
    repo.save(&stored, &[], clock.now()).await?;
    Ok(())
}

/// Flags the sheet for a retry after a failed write.
pub async fn mark_sheet_dirty(
    repo: &impl DealRepository,
    clock: &impl Clock,
    deal: &DealId,
) -> Result<(), AppError> {
    let mut stored = load_deal(repo, deal).await?;
    if stored.sheet_dirty {
        return Ok(());
    }
    stored.sheet_dirty = true;
    repo.save(&stored, &[], clock.now()).await?;
    Ok(())
}
