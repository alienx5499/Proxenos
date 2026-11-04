//! Query: get_status, the status line fields.

use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;
use serde::Serialize;

use super::{StateName, label};
use crate::error::AppError;
use crate::ports::DealReader;
use crate::stored::{Check, load_deal};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub deal_id: DealId,
    pub kind: DealKind,
    pub state: StateName,
    pub round: u16,
    pub max_rounds: u16,
    /// Whose move it is, while open.
    pub turn: Option<String>,
    pub deadline: Timestamp,
    /// Sides that have signed.
    pub signed: Vec<String>,
    /// First 8 hex characters of the agreed terms hash.
    pub hash8: Option<String>,
    /// Own probes left; only when asked from that side's desk room.
    pub probes_left: Option<u8>,
    /// Own recent checks, newest last; only when asked from that side's desk room.
    pub last_checks: Option<Vec<Check>>,
}

pub async fn get_status(
    repo: &impl DealReader,
    deal: &DealId,
    desk_of: Option<Role>,
) -> Result<Status, AppError> {
    let stored = load_deal(repo, deal).await?;
    let d = &stored.deal;
    let kind = d.kind();
    let hash8 = match d.state() {
        DealState::Agreed(h) | DealState::PartlySigned(h) | DealState::Bound(h) => Some(h.short()),
        _ => None,
    };
    Ok(Status {
        deal_id: d.id().clone(),
        kind,
        state: d.state().into(),
        round: d.round(),
        max_rounds: d.max_rounds(),
        turn: (d.state() == DealState::Open).then(|| label(kind, d.turn())),
        deadline: d.deadline(),
        signed: d.signatures().iter().map(|s| label(kind, s.role)).collect(),
        hash8,
        probes_left: desk_of.map(|r| stored.party(r).probes_left(d.round())),
        last_checks: desk_of.map(|r| stored.party(r).checks.clone()),
    })
}
