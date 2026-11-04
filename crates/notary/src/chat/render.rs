//! Offer cards, outcome lines, terms card, desk replies as Mattermost markdown.
//!
//! No output carries an `@` except `wake`, the one deliberate mention: free text has `@`
//! removed, and JSON blocks escape it as `@` (and backticks as ```, so a value
//! cannot close the fence).

use proxenos_domain::date::Date;
use proxenos_domain::deal::{Deal, DealState};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::kind::{DealKind, ValueType};
use proxenos_domain::limit::OwnVerdict;
use proxenos_domain::offer::{Offer, OfferId};
use proxenos_domain::party::Role;
use proxenos_domain::terms::Terms;
use serde::Serialize;
use serde_json::{Value, json};

use super::format;
use super::grammar::ParseError;

pub use proxenos_app::queries::label;

fn clean(text: &str) -> String {
    text.replace('@', "")
}

pub fn deal_open(id: &DealId, kind: DealKind) -> String {
    format!("{id} open · {} · waiting for seals", kind.name())
}

pub fn desk_recorded(id: &DealId, kind: DealKind, role: Role) -> String {
    format!("desk for {} of {id}", label(kind, role))
}

pub fn bound(id: &DealId, kind: DealKind, role: Role) -> String {
    format!("bound: {} of {id}", label(kind, role))
}

pub fn joined(kind: DealKind, role: Role) -> String {
    format!("{} joined", label(kind, role))
}

pub fn negotiator_bound(state: &impl Serialize, today: Date) -> String {
    format!("negotiator bound\n{}", state_block(state, today))
}

/// The only renderer that mentions anyone: `@<agent> <text>`. The name keeps only
/// `[A-Za-z0-9._-]` and the text loses any `@`, so exactly one mention is made.
pub fn wake(agent: &str, text: &str) -> String {
    let name: String = agent
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    format!("@{name} {}", clean(text))
}

pub fn wake_to_bind(agent: &str) -> String {
    wake(agent, "send state to bind")
}

pub fn wake_turn(agent: &str, state: &impl Serialize, today: Date) -> String {
    wake(agent, &format!("your turn\n{}", state_block(state, today)))
}

/// A desk refusal to the desk's own negotiator: the error block, then its `state` if open.
pub fn wake_refused(agent: &str, error_block: &str, state: Option<String>) -> String {
    let state = state.map_or_else(String::new, |s| format!("\n{s}"));
    wake(
        agent,
        &format!("your command was refused\n{error_block}{state}"),
    )
}

/// The one wake after the refusal cap: the agent keeps hitting its own limit this round.
pub fn wake_hold(agent: &str, error_block: &str, state: Option<String>) -> String {
    let state = state.map_or_else(String::new, |s| format!("\n{s}"));
    wake(
        agent,
        &format!(
            "your command was refused again. If your offers keep failing on your owner's limits, \
             repeat your last offer that went through, or accept the other side's latest offer \
             if it suits your owner. This is the last refusal wake this round.\n{error_block}{state}"
        ),
    )
}

/// A party's `state` block: dates as `YYYY-MM-DD` and a `today` field.
pub fn state_block(state: &impl Serialize, today: Date) -> String {
    let mut value = serde_json::to_value(state).unwrap_or_else(|_| json!({"error": "Internal"}));
    iso_dates(&mut value);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("today".into(), json!(today.iso()));
    }
    desk_json(&value)
}

/// Rewrites day counts under date field names, at any depth, to `YYYY-MM-DD`.
fn iso_dates(value: &mut Value) {
    let is_date = |key: &str| {
        [DealKind::PurchaseOrder, DealKind::Lease].iter().any(|k| {
            k.schema()
                .iter()
                .any(|s| s.value_type == ValueType::Date && s.field.name() == key)
        })
    };
    match value {
        Value::Object(obj) => {
            for (key, v) in obj.iter_mut() {
                match v.as_i64() {
                    Some(days) if is_date(key) => *v = json!(Date::from_days(days as i32).iso()),
                    _ => iso_dates(v),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(iso_dates),
        _ => {}
    }
}

/// The counterparty note is quoted as a `counterparty_note` field, on one line.
pub fn offer_card(kind: DealKind, offer: &Offer) -> String {
    let mut card = format!(
        "**Offer {}** · {} · round {}\n{}",
        offer.id,
        label(kind, offer.by),
        offer.round,
        format::terms(&offer.terms)
    );
    let note = clean(offer.note.as_str()).replace(['\n', '\r'], " ");
    if !note.trim().is_empty() {
        let note = escape_markdown(note.trim());
        card.push_str(&format!("\n> counterparty_note: \"{note}\""));
    }
    card
}

/// Backslash-escapes Mattermost markdown metacharacters, and a leading list marker, so
/// counterparty text renders as plain text.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        let meta = matches!(
            c,
            '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '(' | ')' | '#' | '>' | '|' | '!' | '<'
        );
        if meta || (i == 0 && matches!(c, '-' | '+')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn offer_refused() -> String {
    "offer refused by notary".into()
}

pub fn accept_refused() -> String {
    "accept refused by notary".into()
}

/// Names the sign command without an `@`, so the card itself addresses nobody.
pub fn terms_card(id: &DealId, terms: &Terms, hash8: &str) -> String {
    format!(
        "**Terms agreed · {id}** · `{hash8}`\n{}\nOwners sign by sending the notary `sign {hash8}`",
        format::terms(terms)
    )
}

pub fn signed(kind: DealKind, role: Role, hash8: &str) -> String {
    format!("{} signed {hash8}", label(kind, role))
}

pub fn not_a_party(id: &DealId) -> String {
    format!("not a party to {id}")
}

pub fn withdrawn(id: &DealId, kind: DealKind, role: Role) -> String {
    format!("{id} withdrawn by {}", label(kind, role))
}

/// `D-0007 · purchase-order · Open · round 3/8 · turn: seller · deadline 18:00 · signatures: none`.
/// The deadline is UTC; the turn shows only while the deal is open.
pub fn status_line(deal: &Deal) -> String {
    let kind = deal.kind();
    let state = match deal.state() {
        DealState::AwaitingSeals => "waiting for seals",
        DealState::Open => "Open",
        DealState::Agreed(_) => "Agreed",
        DealState::PartlySigned(_) => "partly signed",
        DealState::Bound(_) => "Bound",
        DealState::Withdrawn => "Withdrawn",
        DealState::Expired => "Expired",
    };
    let mut line = format!(
        "{} · {} · {state} · round {}/{}",
        deal.id(),
        kind.name(),
        deal.round(),
        deal.max_rounds()
    );
    if deal.state() == DealState::Open {
        line.push_str(&format!(" · turn: {}", label(kind, deal.turn())));
    }
    let signers: Vec<String> = deal
        .signatures()
        .iter()
        .map(|s| label(kind, s.role))
        .collect();
    let signers = if signers.is_empty() {
        "none".into()
    } else {
        signers.join(", ")
    };
    let deadline = format::time_of_day(deal.deadline());
    line + &format!(" · deadline {deadline} · signatures: {signers}")
}

/// Desk-room status: the deal line plus own probes left and the last own check.
pub fn desk_status(deal: &Deal, probes_left: u32, last: Option<&OwnVerdict>) -> String {
    let check = match last {
        None => "none".to_string(),
        Some(v) if v.within => "within".to_string(),
        Some(v) => {
            let fields: Vec<&str> = v.failing_fields.iter().map(|f| f.name()).collect();
            format!("outside ({})", fields.join(", "))
        }
    };
    format!(
        "{} · probes left: {probes_left} · last check: {check}",
        status_line(deal)
    )
}

/// A fenced JSON block for desk replies.
pub fn desk_json(value: &impl Serialize) -> String {
    let text = serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| r#"{"error":"Internal"}"#.into())
        .replace('@', "\\u0040")
        .replace('`', "\\u0060");
    format!("```json\n{text}\n```")
}

pub fn probe_result(verdict: &OwnVerdict, probes_left: u32) -> String {
    desk_json(&json!({
        "within": verdict.within,
        "failing_fields": verdict.failing_fields,
        "probes_left": probes_left,
    }))
}

pub fn offer_recorded(id: OfferId) -> String {
    desk_json(&json!({ "offer_id": id }))
}

pub fn agreed(hash8: &str) -> String {
    desk_json(&json!({ "agreed": hash8 }))
}

/// `{error: <variant>, message}`, plus `failing_fields` for `OutsideOwnLimits`.
pub fn desk_error(err: &DomainError) -> String {
    // The variant name is the stable code agents match on, e.g. `NotYourTurn`.
    let debug = format!("{err:?}");
    let code = debug.split('(').next().unwrap_or(&debug);
    let mut body = json!({ "error": code, "message": err.to_string() });
    if let DomainError::OutsideOwnLimits(fields) = err {
        body["failing_fields"] = json!(fields);
    }
    desk_json(&body)
}

pub fn parse_error(err: &ParseError) -> String {
    desk_json(&json!({ "error": "BadCommand", "message": err.to_string() }))
}

/// Field schema per kind, for the `kinds` command.
pub fn kinds() -> String {
    let schema = |kind: DealKind| -> Value {
        let fields: serde_json::Map<String, Value> = kind
            .schema()
            .iter()
            .map(|s| {
                let ty = match s.value_type {
                    ValueType::Money => r#"money {"minor": integer, "currency": "USD"}"#,
                    ValueType::Quantity(_) => r#"quantity {"value": integer, "unit": "kg"}"#,
                    ValueType::Date => "date: calendar date string \"YYYY-MM-DD\"",
                    ValueType::Integer => "integer",
                };
                (s.field.name().to_string(), json!(ty))
            })
            .collect();
        json!({
            "roles": [label(kind, Role::A), label(kind, Role::B)],
            "fields": fields,
        })
    };
    desk_json(&json!({
        "purchase-order": schema(DealKind::PurchaseOrder),
        "lease": schema(DealKind::Lease),
        "dates": "dates are written YYYY-MM-DD; state shows today",
    }))
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "render_state_tests.rs"]
mod state_tests;
