//! The notary's JSON shapes for field values: the answer key holds plain numbers (money in
//! major units), and the seal call, the agents' terms and arm D's limits use the domain shape.
//! Agents write dates as `YYYY-MM-DD`; the answer key keeps day counts.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use proxenos_domain::date::Date;
use proxenos_domain::timestamp::Timestamp;
use serde_json::{Map, Value, json};

use crate::BenchError;
use crate::prereg::{Constraint, number};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Money,
    QuantityKg,
    Date,
    Integer,
}

/// Field names are unique across kinds, so the name alone gives the type.
pub fn field_type(field: &str) -> Option<FieldType> {
    Some(match field {
        "price_per_kg" | "monthly_rent" => FieldType::Money,
        "quantity_kg" => FieldType::QuantityKg,
        "deliver_by" | "move_in_by" => FieldType::Date,
        "payment_net_days" | "deposit_months" | "term_months" => FieldType::Integer,
        _ => return None,
    })
}

/// Minor units per major unit for the supported currencies.
pub const MINOR_PER_MAJOR: f64 = 100.0;

pub fn check_currency(code: &str) -> Result<(), BenchError> {
    match code {
        "INR" | "USD" => Ok(()),
        other => Err(BenchError::Input(format!(
            "currency {other} is not INR or USD"
        ))),
    }
}

fn whole(field: &str, v: &Value, scale: f64) -> Result<i64, BenchError> {
    let n = number(v).ok_or_else(|| BenchError::Input(format!("{field}: {v} is not a number")))?
        * scale;
    // 9.37 * 100 is 936.999…, so measure the distance to the nearest whole, not `fract()`.
    if (n - n.round()).abs() > 1e-6 || n < 0.0 {
        return Err(BenchError::Input(format!(
            "{field}: {v} is not a whole, non-negative amount"
        )));
    }
    Ok(n.round() as i64)
}

/// `FieldValue` as the domain serializes it.
fn field_value(field: &str, t: FieldType, v: &Value, currency: &str) -> Result<Value, BenchError> {
    Ok(match t {
        FieldType::Money => {
            json!({"money": {"minor": whole(field, v, MINOR_PER_MAJOR)?, "currency": currency}})
        }
        FieldType::QuantityKg => {
            json!({"quantity": {"value": whole(field, v, 1.0)?, "unit": "kg"}})
        }
        FieldType::Date => json!({"date": whole(field, v, 1.0)?}),
        FieldType::Integer => json!({"integer": whole(field, v, 1.0)?}),
    })
}

/// One side's answer-key limits as the seal endpoint's `limits` object (domain `Constraint`s).
pub fn limits(limits: &BTreeMap<String, Constraint>, currency: &str) -> Result<Value, BenchError> {
    let mut out = Map::new();
    for (field, c) in limits {
        let t =
            field_type(field).ok_or_else(|| BenchError::Input(format!("unknown field {field}")))?;
        let fv = |v: &Value| field_value(field, t, v, currency);
        // Date bounds of NoLaterThan / NoEarlierThan are a bare `Date`, not a `FieldValue`.
        let date = |v: &Value| whole(field, v, 1.0).map(Value::from);
        let c = match c {
            Constraint::AtMost(v) => json!({"at_most": fv(v)?}),
            Constraint::AtLeast(v) => json!({"at_least": fv(v)?}),
            Constraint::NoLaterThan(v) => json!({"no_later_than": date(v)?}),
            Constraint::NoEarlierThan(v) => json!({"no_earlier_than": date(v)?}),
            Constraint::Between(lo, hi) => json!({"between": [fv(lo)?, fv(hi)?]}),
        };
        out.insert(field.clone(), c);
    }
    Ok(Value::Object(out))
}

/// Arm D's limits as the agent reads them: `limits` with dates as `YYYY-MM-DD`.
pub fn limits_for_agent(
    limits_: &BTreeMap<String, Constraint>,
    currency: &str,
) -> Result<Value, BenchError> {
    fn iso(v: &mut Value) {
        match v {
            Value::Number(n) => {
                if let Some(days) = n.as_i64() {
                    *v = json!(Date::from_days(days as i32).iso());
                }
            }
            Value::Array(items) => items.iter_mut().for_each(iso),
            Value::Object(obj) => obj.values_mut().for_each(iso),
            _ => {}
        }
    }
    let mut out = limits(limits_, currency)?;
    if let Some(obj) = out.as_object_mut() {
        for (field, c) in obj.iter_mut() {
            if field_type(field) == Some(FieldType::Date) {
                iso(c);
            }
        }
    }
    Ok(out)
}

/// Today's UTC calendar day from the system clock.
pub fn today() -> Date {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    Date::containing(Timestamp(secs))
}

/// A field value in answer-key units, from a plain number, a `YYYY-MM-DD` date (as its day
/// count), or any domain shape (`{"minor","currency"}`, `{"value","unit"}`, or tagged
/// `{"money": …}` and the like).
pub fn scalar(v: &Value) -> Option<f64> {
    if let Some(d) = v.as_str().and_then(|s| Date::from_iso(s.trim())) {
        return Some(f64::from(d.days()));
    }
    if let Some(n) = number(v) {
        return Some(n);
    }
    let o = v.as_object()?;
    if let Some(m) = o.get("minor") {
        return number(m).map(|m| m / MINOR_PER_MAJOR);
    }
    if let Some(q) = o.get("value") {
        return number(q);
    }
    match o.iter().next() {
        Some((tag, inner))
            if o.len() == 1
                && matches!(tag.as_str(), "money" | "quantity" | "date" | "integer") =>
        {
            scalar(inner)
        }
        _ => None,
    }
}

/// An example terms object for `kind` in the shape the notary parses, for the prompts;
/// any kind other than `lease` gets the purchase-order example.
pub fn terms_example(kind: &str, date: Date) -> String {
    match kind {
        "lease" => json!({
            "monthly_rent": {"minor": 185_000, "currency": "USD"},
            "deposit_months": 1,
            "move_in_by": date.iso(),
            "term_months": 12,
        }),
        _ => json!({
            "price_per_kg": {"minor": 1_340, "currency": "USD"},
            "quantity_kg": {"value": 400, "unit": "kg"},
            "deliver_by": date.iso(),
            "payment_net_days": 40,
        }),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prereg::tests::example_dir;
    use crate::prereg::{Side, load};
    use proxenos_domain::limit::LimitSet;

    #[test]
    fn converted_limits_deserialize_as_domain_limit_sets() {
        let (dir, sha) = example_dir("domain");
        let pre = load(&dir, &sha).unwrap();
        for (id, ans) in &pre.key.scenarios {
            for (side, role) in [(Side::Buyer, "A"), (Side::Seller, "B")] {
                let l = limits(&ans.limits[&side], &pre.key.currency).unwrap();
                let set = json!({"kind": "purchase-order", "role": role, "limits": l});
                let parsed: Result<LimitSet, _> = serde_json::from_value(set.clone());
                assert!(parsed.is_ok(), "{id} {side}: {set} -> {parsed:?}");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cent_amounts_with_float_error_convert_to_exact_minor_units() {
        for (major, minor) in [(9.37, 937), (0.29, 29), (22.80, 2280), (1.005e2, 10050)] {
            let l = BTreeMap::from([("price_per_kg".into(), Constraint::AtMost(json!(major)))]);
            let v = limits(&l, "USD").unwrap();
            assert_eq!(v["price_per_kg"]["at_most"]["money"]["minor"], json!(minor));
        }
    }

    #[test]
    fn agent_limits_show_dates_iso_and_keep_other_values() {
        let l = BTreeMap::from([
            ("deliver_by".into(), Constraint::NoLaterThan(json!(140))),
            ("payment_net_days".into(), Constraint::AtLeast(json!(30))),
        ]);
        let v = limits_for_agent(&l, "INR").unwrap();
        assert_eq!(
            v["deliver_by"],
            json!({"no_later_than": Date::from_days(140).iso()})
        );
        assert_eq!(v["payment_net_days"], json!({"at_least": {"integer": 30}}));
    }

    #[test]
    fn prompt_example_uses_an_iso_date_and_scales_back() {
        let day = Date::from_days(130);
        let t: Value = serde_json::from_str(&terms_example("purchase-order", day)).unwrap();
        assert_eq!(t["deliver_by"], json!(day.iso()));
        assert_eq!(scalar(&t["deliver_by"]), Some(130.0));
        assert_eq!(scalar(&t["price_per_kg"]), Some(13.40));
        assert_eq!(scalar(&t["quantity_kg"]), Some(400.0));
        assert_eq!(scalar(&json!("not-a-date")), None);
        let lease: Value = serde_json::from_str(&terms_example("lease", day)).unwrap();
        assert_eq!(lease["move_in_by"], json!(day.iso()));
        assert_eq!(scalar(&lease["monthly_rent"]), Some(1850.0));
        assert!(lease.get("price_per_kg").is_none());
        assert_eq!(
            scalar(&json!({"money": {"minor": 1250, "currency": "INR"}})),
            Some(12.5)
        );
        assert!(
            limits(
                &BTreeMap::from([("price_per_kg".into(), Constraint::AtMost(json!(4.255)))]),
                "INR"
            )
            .is_err()
        );
    }
}
