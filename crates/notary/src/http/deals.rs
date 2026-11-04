//! Operator endpoints: `POST /api/deals` (bench and integration tests) and `/admit`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use proxenos_app::commands::admit_parties::{AdmitParties, admit_parties};
use proxenos_app::commands::open_deal::{OpenDeal, open_deal_in};
use proxenos_app::queries::label;
use proxenos_app::stored::PendingBinding;
use proxenos_domain::deal::DEFAULT_OPENER;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{ApiError, AppState, HttpPorts};
use proxenos_app::AppError;
use proxenos_app::ports::{Clock, DealReader};

/// Compares SHA-256 digests byte by byte without an early exit, so timing reveals neither
/// the token's length nor where the first difference is.
fn same(a: &[u8], b: &[u8]) -> bool {
    let (a, b) = (Sha256::digest(a), Sha256::digest(b));
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

pub(super) fn require_operator(headers: &HeaderMap, token: &str) -> Result<(), ApiError> {
    let given = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if token.is_empty() || !same(given.as_bytes(), token.as_bytes()) {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

/// The side a request key names: the kind's own label, or buyer/seller for either kind.
fn role_of(kind: DealKind, key: &str) -> Option<Role> {
    let key = key.to_ascii_lowercase();
    [Role::A, Role::B].into_iter().find(|r| {
        key == label(kind, *r)
            || key
                == match r {
                    Role::A => "buyer",
                    Role::B => "seller",
                }
    })
}

#[derive(Deserialize)]
struct PartyIds {
    owner_id: ParticipantId,
    agent_id: ParticipantId,
}

#[derive(Deserialize)]
pub(super) struct OpenBody {
    deal_id: DealId,
    kind: DealKind,
    room_id: RoomId,
    desks: BTreeMap<String, RoomId>,
    max_rounds: Option<u16>,
    parties: Option<BTreeMap<String, PartyIds>>,
}

/// Both sides, keyed by role, each with the request key it came under.
fn by_role<T>(
    kind: DealKind,
    map: BTreeMap<String, T>,
    what: &str,
) -> Result<[(String, T); 2], ApiError> {
    let mut slots: [Option<(String, T)>; 2] = [None, None];
    for (key, v) in map {
        let role = role_of(kind, &key)
            .ok_or_else(|| ApiError::bad_request(format!("unknown side {key} in {what}")))?;
        let slot = &mut slots[usize::from(role == Role::B)];
        if slot.is_some() {
            return Err(ApiError::bad_request(format!("{what} names a side twice")));
        }
        *slot = Some((key, v));
    }
    match slots {
        [Some(a), Some(b)] => Ok([a, b]),
        _ => Err(ApiError::bad_request(format!("{what} needs both sides"))),
    }
}

/// Opens the deal and records both desks in one save; party ids are stored on the deal and
/// bound when each side seals.
pub(super) async fn open<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    headers: HeaderMap,
    body: Result<Json<OpenBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    require_operator(&headers, &s.cfg.operator_token)?;
    let Json(body) = body?;
    let kind = body.kind;
    let desks = by_role(kind, body.desks, "desks")?;
    let parties = body
        .parties
        .map(|p| by_role(kind, p, "parties"))
        .transpose()?;
    if let Some([(_, a), (_, b)]) = &parties {
        let ids = [&a.owner_id, &a.agent_id, &b.owner_id, &b.agent_id];
        let distinct = (0..4).all(|i| (i + 1..4).all(|j| ids[i] != ids[j]));
        if !distinct {
            return Err(ApiError::bad_request("party ids must all differ"));
        }
        if [&a.agent_id, &b.agent_id]
            .iter()
            .any(|id| s.cfg.excluded.contains(id))
        {
            return Err(AppError::ExcludedNegotiator.into());
        }
    }
    let p = &s.ports;
    let now = p.clock().now();
    let cmd = OpenDeal {
        id: body.deal_id.clone(),
        kind,
        room: body.room_id,
        deadline: Timestamp(now.seconds().saturating_add(s.cfg.deal_secs)),
        max_rounds: body.max_rounds.unwrap_or(s.cfg.max_rounds),
        opener: DEFAULT_OPENER,
        parties: parties.map(|sides| {
            sides.map(|(_, ids)| PendingBinding {
                owner: ids.owner_id,
                agent: ids.agent_id,
            })
        }),
    };
    let rooms = [desks[0].1.clone(), desks[1].1.clone()];
    let tokens = open_deal_in(p.repo(), p.clock(), p.secrets(), cmd, Some(rooms)).await?;
    let link = |t: &str| format!("{}/seal/{t}", s.cfg.public_url);
    let [(key_a, _), (key_b, _)] = desks;
    Ok(Json(json!({
        "deal_id": body.deal_id,
        "seal_links": { key_a: link(&tokens.role_a), key_b: link(&tokens.role_b) },
    })))
}

#[derive(Deserialize, Default)]
struct AdmitBody {
    group: Option<String>,
    /// Gateway usernames (or ExternalUser ids) of both owners.
    owners: Option<Vec<String>>,
    /// Agent store ids of both negotiators.
    agents: Option<Vec<String>>,
}

/// Both owners and both agents must be named; the group defaults to the one whose name
/// starts with the deal id.
pub(super) async fn admit<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    require_operator(&headers, &s.cfg.operator_token)?;
    let deal: DealId = id.parse()?;
    let body: AdmitBody = if body.iter().all(u8::is_ascii_whitespace) {
        AdmitBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let p = &s.ports;
    p.repo()
        .load(&deal)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found(format!("no deal {deal}")))?;
    // Bound ids are `sender` ids; the policy needs Gateway usernames and agent store ids,
    // so nothing is inferred from the deal.
    let (Some(owners), Some(agents)) = (
        body.owners.filter(|v| v.len() == 2),
        body.agents.filter(|v| v.len() == 2),
    ) else {
        return Err(ApiError::bad_request(
            "name both owners' Gateway usernames and both negotiators' agent ids: \
             proxenos admit <deal> --owner <user> --owner <user> --agent <id> --agent <id>",
        ));
    };
    let cmd = AdmitParties {
        deal: deal.clone(),
        group_name: body.group.unwrap_or_else(|| deal.to_string()),
        owner_usernames: owners,
        agent_ids: agents,
    };
    let group = admit_parties(p.repo(), p.admin(), p.clock(), cmd).await?;
    Ok(Json(json!({ "deal_id": deal, "group_id": group })))
}
