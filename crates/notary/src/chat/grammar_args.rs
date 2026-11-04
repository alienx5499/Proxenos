//! Argument parsers for the verbs that take arguments: kickoffs, sign, probe, offer.

use proxenos_domain::date::Date;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::{DealKind, ValueType};
use proxenos_domain::offer::Note;
use proxenos_domain::terms::{LeaseTerms, PurchaseOrderTerms, Terms};
use serde::Deserialize;
use serde_json::Value;

use super::{Command, ParseError, SideLabel, one_word};

fn deal_id(word: &str) -> Result<DealId, ParseError> {
    word.parse().map_err(|_| ParseError::BadDealId)
}

/// `open <kind> deal <D-dddd> [opener <side>]`; the rest of the message is ignored.
pub(super) fn open(args: &str) -> Result<Command, ParseError> {
    const USAGE: &str = "open <kind> deal <D-dddd> [opener <side>]";
    let words: Vec<&str> = args.split_whitespace().take(5).collect();
    let [kind, deal, id, ..] = words[..] else {
        return Err(ParseError::Usage(USAGE));
    };
    let opener = match words[3..] {
        [word, side] if word.eq_ignore_ascii_case("opener") => Some(side_label(side)?),
        _ => None,
    };
    if !deal.eq_ignore_ascii_case("deal") {
        return Err(ParseError::Usage(USAGE));
    }
    let kind = match kind.to_ascii_lowercase().as_str() {
        "purchase-order" => DealKind::PurchaseOrder,
        "lease" => DealKind::Lease,
        _ => return Err(ParseError::BadKind),
    };
    Ok(Command::Open {
        kind,
        deal_id: deal_id(id)?,
        opener,
    })
}

/// `desk <side> of <D-dddd>`.
pub(super) fn desk(args: &str) -> Result<Command, ParseError> {
    const USAGE: &str = "desk <side> of <D-dddd>";
    let [side, of, id] = args.split_whitespace().collect::<Vec<_>>()[..] else {
        return Err(ParseError::Usage(USAGE));
    };
    if !of.eq_ignore_ascii_case("of") {
        return Err(ParseError::Usage(USAGE));
    }
    Ok(Command::Desk {
        side: side_label(side)?,
        deal_id: deal_id(id)?,
    })
}

fn side_label(word: &str) -> Result<SideLabel, ParseError> {
    Ok(match word.to_ascii_lowercase().as_str() {
        "buyer" => SideLabel::Buyer,
        "seller" => SideLabel::Seller,
        "tenant" => SideLabel::Tenant,
        "landlord" => SideLabel::Landlord,
        "a" => SideLabel::A,
        "b" => SideLabel::B,
        _ => return Err(ParseError::BadSide),
    })
}

pub(super) fn sign(args: &str) -> Result<Command, ParseError> {
    let hash = one_word(args, "sign <hash8>")?;
    if hash.len() != 8 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParseError::BadHash);
    }
    Ok(Command::Sign {
        hash8: hash.to_ascii_lowercase(),
    })
}

/// `probe {terms}`; a `{"terms": {...}}` wrapper is accepted too.
pub(super) fn probe(args: &str) -> Result<Command, ParseError> {
    let mut value = json(args)?;
    if let Some(obj) = value.as_object_mut()
        && obj.len() == 1
        && let Some(inner) = obj.remove("terms")
    {
        value = inner;
    }
    Ok(Command::Probe {
        terms: terms(value)?,
    })
}

/// `offer {"terms": {...}, "note": "..."}`; the note is optional.
pub(super) fn offer(args: &str) -> Result<Command, ParseError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Body {
        terms: Value,
        #[serde(default)]
        note: Note,
    }
    let body: Body = serde_json::from_value(json(args)?).map_err(bad_json)?;
    Ok(Command::Offer {
        terms: terms(body.terms)?,
        note: body.note,
    })
}

fn bad_json(e: serde_json::Error) -> ParseError {
    ParseError::BadJson(e.to_string())
}

/// Strips a code fence (with optional language tag) or inline backticks, then parses.
fn json(args: &str) -> Result<Value, ParseError> {
    let mut s = args.trim();
    if let Some(fenced) = s.strip_prefix("```") {
        let body = fenced
            .trim_start_matches(|c: char| c.is_ascii_alphanumeric())
            .trim();
        s = body.strip_suffix("```").unwrap_or(body).trim();
    } else {
        s = s.trim_matches('`').trim();
    }
    if s.is_empty() {
        return Err(ParseError::Usage(
            "probe {terms} or offer {\"terms\":{...}}",
        ));
    }
    serde_json::from_str(s).map_err(bad_json)
}

/// With `kind` the tagged form is used; without it, the first field name picks the kind.
fn terms(mut value: Value) -> Result<Terms, ParseError> {
    iso_dates_to_days(&mut value)?;
    let obj = value.as_object().ok_or(ParseError::UnknownTerms)?;
    let has = |kind: DealKind| {
        kind.schema()
            .iter()
            .any(|s| obj.contains_key(s.field.name()))
    };
    let parsed = if obj.contains_key("kind") {
        serde_json::from_value(value)
    } else if has(DealKind::PurchaseOrder) {
        serde_json::from_value::<PurchaseOrderTerms>(value).map(Terms::PurchaseOrder)
    } else if has(DealKind::Lease) {
        serde_json::from_value::<LeaseTerms>(value).map(Terms::Lease)
    } else {
        return Err(ParseError::UnknownTerms);
    };
    parsed.map_err(bad_json)
}

/// Rewrites `YYYY-MM-DD` strings in date fields to the domain's day count.
fn iso_dates_to_days(value: &mut Value) -> Result<(), ParseError> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    for kind in [DealKind::PurchaseOrder, DealKind::Lease] {
        for spec in kind
            .schema()
            .iter()
            .filter(|s| s.value_type == ValueType::Date)
        {
            if let Some(v) = obj.get_mut(spec.field.name())
                && let Value::String(s) = v
            {
                let date = Date::from_iso(s.trim()).ok_or(ParseError::BadDate)?;
                *v = date.days().into();
            }
        }
    }
    Ok(())
}
