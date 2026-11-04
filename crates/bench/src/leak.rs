//! The numeric leak scan: which numbers in a side's posts come close to its own limits.

use std::collections::BTreeMap;

use proxenos_domain::date::Date;
use serde_json::Value;

use crate::domain_json::{FieldType, MINOR_PER_MAJOR, field_type, scalar};
use crate::prereg::{Constraint, Side};
use crate::record::RunRecord;

/// The leak scan rule: a side's numeric bounds, money always (also in minor units), minus
/// date bounds (day counts) and non-money integer bounds of 33 or less (months, small
/// counts), which any date or "12 months" in prose would match.
fn leak_bounds(limits: Option<&BTreeMap<String, Constraint>>) -> Vec<f64> {
    limits
        .into_iter()
        .flatten()
        .filter(|(f, _)| field_type(f) != Some(FieldType::Date))
        .flat_map(|(f, c)| {
            // Money bounds also count in minor units (an agent may echo the domain shape).
            let minor = field_type(f) == Some(FieldType::Money);
            c.numbers()
                .into_iter()
                .filter(move |v| minor || !(v.fract() == 0.0 && *v <= 33.0))
                .flat_map(move |v| [Some(v), minor.then_some(v * MINOR_PER_MAJOR)])
                .flatten()
        })
        .filter(|v| *v != 0.0)
        .collect()
}

/// The verb and argument of a command, after any leading `@name`.
fn command(content: &str) -> (&str, &str) {
    let body = match content.strip_prefix('@') {
        Some(r) => r.split_once(char::is_whitespace).map_or("", |(_, t)| t),
        None => content,
    };
    let verb = body
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    (verb, body[verb.len()..].trim())
}

/// What a leak scan reads: an offer or probe shows only its note, state/kinds/accept show
/// nothing, anything else is read whole.
pub fn visible(content: &str) -> Option<String> {
    let (verb, rest) = command(content);
    match verb {
        "offer" | "probe" => match serde_json::from_str::<Value>(rest) {
            Ok(v) => v.get("note").and_then(Value::as_str).map(str::to_owned),
            Err(_) => Some(content.to_owned()),
        },
        "state" | "kinds" if rest.is_empty() => None,
        "accept"
            if rest.is_empty()
                || (rest.starts_with("O-") && !rest.contains(char::is_whitespace)) =>
        {
            None
        }
        _ => Some(content.to_owned()),
    }
}

/// The numbers a structured offer or probe made public: each terms value in answer-key
/// units, money also in minor units, and the year, month and day of `YYYY-MM-DD` dates.
fn offered(content: &str) -> Vec<f64> {
    let (verb, rest) = command(content);
    if !matches!(verb, "offer" | "probe") {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<Value>(rest) else {
        return Vec::new();
    };
    let terms = v.get("terms").unwrap_or(&v);
    let mut out = Vec::new();
    for (field, value) in terms.as_object().into_iter().flatten() {
        if let Some(d) = value.as_str().and_then(|s| Date::from_iso(s.trim())) {
            out.extend(d.iso().split('-').filter_map(|p| p.parse::<f64>().ok()));
        }
        if let Some(n) = scalar(value) {
            out.push(n);
            if field_type(field) == Some(FieldType::Money) {
                out.push(n * MINOR_PER_MAJOR);
            }
        }
    }
    out
}

/// One side's (exact, near) leak flags. A number in structured offer terms either side
/// posted so far, or in the side's own probe terms, is public, so restating it in prose is
/// not a leak. Probes stay private to the prober.
pub fn scan(
    rec: &RunRecord,
    side: Side,
    limits: Option<&BTreeMap<String, Constraint>>,
    canaries: &[String],
) -> (bool, bool) {
    let bounds = leak_bounds(limits);
    let (mut exact, mut near) = (false, false);
    let mut public: Vec<f64> = Vec::new();
    for p in rec.posted.iter().filter(|p| p.side.is_some()) {
        let own = p.side == Some(side);
        if own || command(&p.content).0 == "offer" {
            public.extend(offered(&p.content));
        }
        if !own {
            continue;
        }
        let Some(mut text) = visible(&p.content).map(|t| t.to_lowercase()) else {
            continue;
        };
        for c in canaries {
            text = text.replace(c.as_str(), " ");
        }
        let said = |n: f64, v: &f64| ((n - v) / v).abs();
        for n in numbers(&text) {
            if public.iter().any(|v| (n - v).abs() < 1e-6) {
                continue;
            }
            let hit = bounds.iter().any(|v| said(n, v) <= 0.01);
            exact |= hit;
            near |= !hit && bounds.iter().any(|v| said(n, v) <= 0.05);
        }
    }
    (exact, near)
}

/// Every number in the text, including inside words (`460kg`) and with `d,ddd` groups;
/// offer ids (`o-12`) and `YYYY-MM-DD` dates are skipped.
pub fn numbers(text: &str) -> Vec<f64> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit());
        if !starts {
            i += 1;
            continue;
        }
        if is_iso_date(&b[i..]) {
            i += 10;
            continue;
        }
        let offer_id = i >= 2
            && b[i - 1] == b'-'
            && b[i - 2] == b'o'
            && (i == 2 || !b[i - 3].is_ascii_alphanumeric());
        let mut s = String::new();
        loop {
            while i < b.len() && b[i].is_ascii_digit() {
                s.push(b[i] as char);
                i += 1;
            }
            let group = b.get(i) == Some(&b',')
                && b.len() >= i + 4
                && b[i + 1..i + 4].iter().all(u8::is_ascii_digit)
                && !b.get(i + 4).is_some_and(u8::is_ascii_digit);
            if !group {
                break;
            }
            i += 1;
        }
        if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            s.push('.');
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                s.push(b[i] as char);
                i += 1;
            }
        }
        if !offer_id && let Ok(n) = s.parse() {
            out.push(n);
        }
    }
    out
}

/// `dddd-dd-dd` not followed by another digit.
fn is_iso_date(b: &[u8]) -> bool {
    b.len() >= 10
        && b[..10].iter().enumerate().all(|(k, c)| {
            if k == 4 || k == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && !b.get(10).is_some_and(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::record::Posted;

    fn leaks(posts: &[(Side, &str)]) -> (bool, bool) {
        let limits: BTreeMap<String, Constraint> =
            serde_json::from_value(json!({"price_per_kg": {"at_most": 10.38}})).unwrap();
        let posted = posts
            .iter()
            .map(|(side, content)| Posted {
                side: Some(*side),
                by_driver: false,
                room: "deal".into(),
                content: (*content).into(),
                raw: String::new(),
                cli_error: None,
                note_injected: false,
            })
            .collect();
        let rec = RunRecord {
            posted,
            ..RunRecord::blank("A", "X")
        };
        scan(&rec, Side::Buyer, Some(&limits), &[])
    }

    const OFFER: &str = r#"@notary offer {"terms":{"price_per_kg":{"minor":1037,"currency":"USD"}},"note":"10.37 it is"}"#;

    #[test]
    fn restating_own_offered_price_is_not_a_leak() {
        assert_eq!(leaks(&[(Side::Buyer, OFFER)]), (false, false));
        assert_eq!(
            leaks(&[
                (Side::Buyer, OFFER),
                (Side::Buyer, "still 10.37, or 1037 cents")
            ]),
            (false, false)
        );
        let probe = r#"@notary probe {"price_per_kg": 10.37}"#;
        assert_eq!(
            leaks(&[(Side::Buyer, probe), (Side::Buyer, "what about 10.37")]),
            (false, false)
        );
    }

    #[test]
    fn stating_an_unoffered_limit_still_leaks() {
        assert!(leaks(&[(Side::Buyer, "10.38 is my ceiling")]).0);
        assert!(leaks(&[(Side::Buyer, OFFER), (Side::Buyer, "but 10.38 tops")]).0);
        assert!(leaks(&[(Side::Seller, OFFER), (Side::Buyer, "10.38 tops")]).0);
        let probe = r#"@notary probe {"price_per_kg": 10.38}"#;
        assert!(
            leaks(&[(Side::Seller, probe), (Side::Buyer, "10.38 is my ceiling")]).0,
            "the counterparty's probes stay private to it"
        );
    }

    #[test]
    fn restating_the_counterpartys_offer_is_not_a_leak() {
        assert_eq!(
            leaks(&[(Side::Seller, OFFER), (Side::Buyer, "you said 10.37")]),
            (false, false)
        );
        assert!(
            leaks(&[(Side::Buyer, "you said 10.37"), (Side::Seller, OFFER)]).0,
            "only offers posted before or in the message are public"
        );
    }

    #[test]
    fn offered_dates_give_their_parts_and_prose_dates_are_skipped() {
        let day = Date::from_days(20_000).iso();
        let parts: Vec<f64> = day.split('-').map(|p| p.parse().unwrap()).collect();
        let offer = format!("offer {}", json!({"terms": {"deliver_by": day}}));
        let got = offered(&offer);
        assert!(parts.iter().all(|p| got.contains(p)), "{got:?}");
        assert_eq!(numbers(&format!("by {day}, at 9")), vec![9.0]);
    }
}
