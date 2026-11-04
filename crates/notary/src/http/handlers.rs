//! Public handlers: health, seal page and API, timeline, receipt, notary key.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use proxenos_app::AppError;
use proxenos_app::commands::seal_limits::{SealLimits, seal_limits};
use proxenos_app::ports::{DealReader, ReceiptSigner};
use proxenos_app::queries::get_receipt::get_receipt;
use proxenos_app::queries::get_seal_form::get_seal_form;
use proxenos_app::queries::get_timeline::get_timeline;
use proxenos_app::queries::{StateName, label};
use proxenos_app::secret::hex;
use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::{Direction, Field, ValueType};
use proxenos_domain::limit::{Constraint, LimitSet};
use proxenos_domain::quantity::Unit;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ApiError, AppState, HttpPorts};

pub async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

fn direction(d: Direction) -> &'static str {
    match d {
        Direction::AtMost => "at_most",
        Direction::AtLeast => "at_least",
        Direction::NoLaterThan => "no_later_than",
        Direction::NoEarlierThan => "no_earlier_than",
        Direction::Between => "between",
    }
}

fn value_type(t: ValueType) -> Value {
    match t {
        ValueType::Money => json!({ "type": "money" }),
        ValueType::Quantity(u) => json!({ "type": "quantity", "unit": match u {
            Unit::Kg => "kg",
            Unit::Months => "months",
        }}),
        ValueType::Date => json!({ "type": "date" }),
        ValueType::Integer => json!({ "type": "integer" }),
    }
}

/// The side's field schema; never a value.
pub async fn seal_form<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    Path(token): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let p = &s.ports;
    let form = get_seal_form(p.repo(), p.secrets(), &token).await?;
    let fields: Vec<Value> = form
        .kind
        .schema()
        .iter()
        .map(|spec| {
            let mut f = value_type(spec.value_type);
            f["field"] = json!(spec.field);
            f["direction"] = json!(direction(spec.direction(form.role)));
            f
        })
        .collect();
    Ok(Json(json!({
        "deal_id": form.deal,
        "kind": form.kind,
        "side": label(form.kind, form.role),
        "currencies": ["USD", "INR"],
        "fields": fields,
    })))
}

#[derive(Deserialize)]
pub struct SealBody {
    limits: BTreeMap<Field, Constraint>,
}

/// Seals; a side whose ids came with `POST /api/deals` is bound in the same save.
pub async fn seal<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    Path(token): Path<String>,
    body: Result<Json<SealBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    let p = &s.ports;
    let form = get_seal_form(p.repo(), p.secrets(), &token).await?;
    let limits = LimitSet::new(form.kind, form.role, body.limits)?;
    let cmd = SealLimits { token, limits };
    let sealed = seal_limits(p.repo(), p.vault(), p.clock(), p.secrets(), cmd).await?;
    if let Some(tx) = &s.opened {
        let stored = p.repo().load(&sealed.deal).await.map_err(AppError::from)?;
        if stored.is_some_and(|st| st.deal.state() == DealState::Open) {
            // The seal is saved; a lost wake is logged, the next status or move recovers.
            if tx.send(sealed.deal.clone()).await.is_err() {
                eprintln!("http: router gone; no turn wake for {}", sealed.deal);
            }
        }
    }
    Ok(Json(json!({
        "deal_id": sealed.deal,
        "side": label(form.kind, sealed.role),
        "join_code": sealed.join_code,
        "bound": sealed.bound,
    })))
}

/// Bench reads `state` with these exact names.
fn state_name(s: StateName) -> &'static str {
    match s {
        StateName::AwaitingSeals => "AwaitingSeals",
        StateName::Open => "Open",
        StateName::Agreed => "Agreed",
        StateName::PartlySigned => "PartlySigned",
        StateName::Bound => "Bound",
        StateName::Withdrawn => "Withdrawn",
        StateName::Expired => "Expired",
    }
}

/// The redacted log plus the deal's state, round and agreed terms (null until Agreed).
pub async fn timeline<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id: DealId = id.parse()?;
    let repo = s.ports.repo();
    let stored = repo
        .load(&id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::DealNotFound(id.clone()))?;
    let entries = get_timeline(repo, &id).await?;
    let deal = &stored.deal;
    Ok(Json(json!({
        "deal_id": id,
        "kind": deal.kind(),
        "state": state_name(deal.state().into()),
        "round": deal.round(),
        "max_rounds": deal.max_rounds(),
        "turn": label(deal.kind(), deal.turn()),
        "deadline": deal.deadline(),
        "agreed_terms": deal.agreed_offer().map(|o| &o.terms),
        "entries": entries,
    })))
}

pub async fn receipt<P: HttpPorts>(
    State(s): State<Arc<AppState<P>>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id: DealId = id.parse()?;
    let receipt = get_receipt(s.ports.repo(), &id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("{id} has no receipt yet")))?;
    Ok(Json(json!(receipt)))
}

pub async fn notary_key<P: HttpPorts>(State(s): State<Arc<AppState<P>>>) -> Json<Value> {
    Json(json!({ "algorithm": "ed25519", "public_key": hex(&s.ports.signer().public_key()) }))
}
