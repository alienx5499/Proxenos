//! Refusal texts for app errors, and the router's outcome lines that `render` lacks.

use proxenos_app::AppError;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use serde_json::json;

use super::render::{self, label};

/// A desk error block `{error, message}` with a stable code.
pub fn desk_message(code: &str, message: &str) -> String {
    render::desk_json(&json!({ "error": code, "message": message }))
}

/// Store, vault, Switch and internal failures are not replies: `None`, so they are returned.
fn user_facing(err: &AppError) -> Option<(String, String)> {
    let debug = format!("{err:?}");
    let code: String = debug
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    match err {
        AppError::Repo(_) | AppError::Vault(_) | AppError::Switch(_) | AppError::Internal(_) => {
            None
        }
        AppError::Domain(d) => {
            let debug = format!("{d:?}");
            let code = debug
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            Some((code, d.to_string()))
        }
        _ => Some((code, err.to_string())),
    }
}

/// For a desk room: the domain error block (with own failing fields), or `{error, message}`.
pub fn desk_app_error(err: &AppError) -> Option<String> {
    if let AppError::Domain(d) = err {
        return Some(render::desk_error(d));
    }
    user_facing(err).map(|(code, message)| desk_message(&code, &message))
}

/// For the deal room: the message only, never a field name.
pub fn refusal_line(err: &AppError) -> Option<String> {
    user_facing(err).map(|(_, message)| message.replace('@', ""))
}

/// After a store or port failure: nothing changed, the sender should send it again.
pub fn not_recorded() -> String {
    "notary error: that command was not recorded; send it again".into()
}

pub fn probe_refused() -> String {
    "probe refused by notary".into()
}

pub fn deal_bound(id: &DealId, hash8: &str, public_url: &str) -> String {
    format!("{id} bound · terms {hash8} · receipt: {public_url}/api/deals/{id}/receipt")
}

pub fn expired(id: &DealId) -> String {
    format!("{id} expired: round cap reached")
}

pub fn declined(kind: DealKind, role: Role) -> String {
    format!("{} declined to sign", label(kind, role))
}

pub fn wrong_party() -> String {
    "answered by the wrong party".into()
}

pub fn card_expired(kind: DealKind, role: Role) -> String {
    format!("sign card for {} expired", label(kind, role))
}

/// A desk kickoff whose side label belongs to the other deal kind.
pub fn side_mismatch(kind: DealKind) -> String {
    let (a, b) = (label(kind, Role::A), label(kind, Role::B));
    format!(
        "side does not fit a {} deal; use {a}, {b}, a or b",
        kind.name()
    )
}
