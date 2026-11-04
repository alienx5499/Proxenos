//! Queries: read only; they take `DealReader` and never mutate.

pub mod get_party_view;
pub mod get_receipt;
pub mod get_seal_form;
pub mod get_status;
pub mod get_timeline;
pub mod list_deals;

use proxenos_domain::deal::DealState;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use serde::Serialize;

/// Deal state without the hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateName {
    AwaitingSeals,
    Open,
    Agreed,
    PartlySigned,
    Bound,
    Withdrawn,
    Expired,
}

impl From<DealState> for StateName {
    fn from(s: DealState) -> Self {
        match s {
            DealState::AwaitingSeals => Self::AwaitingSeals,
            DealState::Open => Self::Open,
            DealState::Agreed(_) => Self::Agreed,
            DealState::PartlySigned(_) => Self::PartlySigned,
            DealState::Bound(_) => Self::Bound,
            DealState::Withdrawn => Self::Withdrawn,
            DealState::Expired => Self::Expired,
        }
    }
}

/// Lowercase side label: buyer, seller, tenant, landlord.
pub fn label(kind: DealKind, role: Role) -> String {
    kind.role_label(role).to_ascii_lowercase()
}
