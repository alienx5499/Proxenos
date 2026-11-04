//! Row mapping between `StoredDeal` / `LoggedEvent` and SQLite columns.

use proxenos_app::ports::RepoError;
use proxenos_app::stored::{LoggedEvent, StoredDeal};
use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;

pub fn backend(e: impl std::fmt::Display) -> RepoError {
    RepoError::Backend(e.to_string())
}

pub fn role_text(role: Role) -> &'static str {
    match role {
        Role::A => "A",
        Role::B => "B",
    }
}

pub fn parse_role(s: &str) -> Result<Role, RepoError> {
    match s {
        "A" => Ok(Role::A),
        "B" => Ok(Role::B),
        other => Err(backend(format!("bad role {other:?} in store"))),
    }
}

pub fn parse_id(s: &str) -> Result<DealId, RepoError> {
    s.parse().map_err(backend)
}

/// State name and the terms hash it carries, if any.
pub fn state_columns(state: DealState) -> (&'static str, Option<String>) {
    match state {
        DealState::AwaitingSeals => ("awaiting_seals", None),
        DealState::Open => ("open", None),
        DealState::Agreed(h) => ("agreed", Some(h.to_string())),
        DealState::PartlySigned(h) => ("partly_signed", Some(h.to_string())),
        DealState::Bound(h) => ("bound", Some(h.to_string())),
        DealState::Withdrawn => ("withdrawn", None),
        DealState::Expired => ("expired", None),
    }
}

/// Decodes the JSON body (the domain re-checks every invariant); the column version wins.
pub fn decode_deal(body: &str, version: i64) -> Result<StoredDeal, RepoError> {
    let mut deal: StoredDeal = serde_json::from_str(body).map_err(backend)?;
    deal.version = u64::try_from(version).map_err(backend)?;
    Ok(deal)
}

pub fn encode_event(event: &DealEvent) -> Result<(String, String), RepoError> {
    let value = serde_json::to_value(event).map_err(backend)?;
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| backend("deal event has no type tag"))?
        .to_owned();
    Ok((kind, value.to_string()))
}

pub fn decode_event(seq: i64, at: i64, payload: &str) -> Result<LoggedEvent, RepoError> {
    Ok(LoggedEvent {
        seq: u64::try_from(seq).map_err(backend)?,
        at: Timestamp(at),
        event: serde_json::from_str(payload).map_err(backend)?,
    })
}
