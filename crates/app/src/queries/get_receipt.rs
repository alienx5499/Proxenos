//! Query: get_receipt, present once the deal is Bound.

use proxenos_domain::deal_id::DealId;

use crate::error::AppError;
use crate::ports::DealReader;
use crate::receipt::Receipt;
use crate::stored::load_deal;

pub async fn get_receipt(
    repo: &impl DealReader,
    deal: &DealId,
) -> Result<Option<Receipt>, AppError> {
    Ok(load_deal(repo, deal).await?.receipt)
}
