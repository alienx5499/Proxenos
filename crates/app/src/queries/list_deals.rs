//! Query: list_deals, deals that use a room as deal room or desk room.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{Role, RoomId};
use serde::Serialize;

use super::StateName;
use crate::error::AppError;
use crate::ports::DealReader;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DealSummary {
    pub id: DealId,
    pub kind: DealKind,
    pub state: StateName,
    /// The side whose desk this room is, if it is a desk room.
    pub desk_of: Option<Role>,
}

pub async fn list_deals(
    repo: &impl DealReader,
    room: &RoomId,
) -> Result<Vec<DealSummary>, AppError> {
    let deals = repo.in_room(room).await?;
    Ok(deals
        .into_iter()
        .map(|s| DealSummary {
            id: s.deal.id().clone(),
            kind: s.deal.kind(),
            state: s.deal.state().into(),
            desk_of: [Role::A, Role::B]
                .into_iter()
                .find(|r| s.deal.desk(*r) == Some(room)),
        })
        .collect())
}
