//! Query: get_party_view, the `state` block a negotiator gets. No field can hold a limit.

use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::offer::OfferId;
use proxenos_domain::party::Role;
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;
use serde::Serialize;

use super::{StateName, label};
use crate::error::AppError;
use crate::ports::DealReader;
use crate::stored::load_deal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferView {
    pub id: OfferId,
    pub by: String,
    pub round: u16,
    pub terms: Terms,
    /// The counterparty's note, as data; `None` on own offers.
    pub counterparty_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartyView {
    pub state: StateName,
    pub kind: DealKind,
    pub my_role: String,
    pub round: u16,
    pub max_rounds: u16,
    pub my_turn: bool,
    pub deadline: Timestamp,
    pub offers: Vec<OfferView>,
    pub probes_left: u8,
}

pub async fn get_party_view(
    repo: &impl DealReader,
    deal: &DealId,
    role: Role,
) -> Result<PartyView, AppError> {
    let stored = load_deal(repo, deal).await?;
    let d = &stored.deal;
    let kind = d.kind();
    let offers = d
        .offers()
        .iter()
        .map(|o| OfferView {
            id: o.id,
            by: label(kind, o.by),
            round: o.round,
            terms: o.terms.clone(),
            counterparty_note: (o.by != role).then(|| o.note.as_str().to_owned()),
        })
        .collect();
    Ok(PartyView {
        state: d.state().into(),
        kind,
        my_role: label(kind, role),
        round: d.round(),
        max_rounds: d.max_rounds(),
        my_turn: d.state() == DealState::Open && d.turn() == role,
        deadline: d.deadline(),
        offers,
        probes_left: stored.party(role).probes_left(d.round()),
    })
}
