//! Date fields in probe and offer terms: ISO strings, legacy day counts, bad dates.

use super::*;
use proxenos_domain::date::Date;

fn deliver_by(cmd: Result<Command, ParseError>) -> Date {
    match cmd {
        Ok(Command::Probe {
            terms: Terms::PurchaseOrder(t),
        })
        | Ok(Command::Offer {
            terms: Terms::PurchaseOrder(t),
            ..
        }) => t.deliver_by(),
        other => panic!("{other:?}"),
    }
}

fn po(date: &str) -> String {
    format!(
        r#"{{"price_per_kg":{{"minor":109000,"currency":"INR"}},"quantity_kg":{{"value":400,"unit":"kg"}},"deliver_by":{date},"payment_net_days":45}}"#
    )
}

#[test]
fn iso_date_parses_to_the_same_day_as_the_count() {
    let day = Date::from_days(20_000);
    let iso = format!("\"{}\"", day.iso());
    assert_eq!(
        deliver_by(parse(&format!("@notary probe {}", po(&iso)))),
        day
    );
    let offer = format!(r#"@notary offer {{"terms":{},"note":"x"}}"#, po(&iso));
    assert_eq!(deliver_by(parse(&offer)), day);
    let legacy = format!("@notary probe {}", po("20000"));
    assert_eq!(deliver_by(parse(&legacy)), day);
}

#[test]
fn lease_move_in_by_takes_iso_too() {
    let day = Date::from_days(321);
    let body = format!(
        r#"@notary probe {{"kind":"lease","monthly_rent":{{"minor":1,"currency":"INR"}},"deposit_months":2,"move_in_by":"{}","term_months":11}}"#,
        day.iso()
    );
    match parse(&body) {
        Ok(Command::Probe {
            terms: Terms::Lease(l),
        }) => assert_eq!(l.move_in_by, day),
        other => panic!("{other:?}"),
    }
}

#[test]
fn impossible_or_malformed_dates_are_bad_dates() {
    let year = Date::from_days(20_000).iso()[..4].to_string();
    for bad in [
        format!("\"{year}-02-30\""),
        format!("\"{year}-13-01\""),
        "\"late\"".into(),
        "\"329\"".into(),
    ] {
        let body = format!("@notary probe {}", po(&bad));
        assert_eq!(parse(&body), Err(ParseError::BadDate), "{bad}");
    }
}
