//! The state block shows calendar dates and today; kinds names the date format.

use super::*;
use proxenos_domain::date::Date;

fn block_json(text: &str) -> Value {
    let body = text
        .split("```json\n")
        .nth(1)
        .and_then(|b| b.split("\n```").next())
        .unwrap();
    serde_json::from_str(body).unwrap()
}

#[test]
fn state_block_renders_dates_iso_and_adds_today() {
    let state = json!({
        "round": 2,
        "offers": [
            {"round": 1, "terms": {"kind": "purchase-order", "deliver_by": 20_000, "payment_net_days": 45}},
            {"terms": {"kind": "lease", "move_in_by": 321, "term_months": 11}},
        ],
    });
    let today = Date::from_days(19_990);
    for text in [
        state_block(&state, today),
        negotiator_bound(&state, today),
        wake_turn("buyer-agent", &state, today),
    ] {
        let v = block_json(&text);
        assert_eq!(v["today"], json!(today.iso()));
        assert_eq!(v["round"], json!(2));
        let offers = &v["offers"];
        assert_eq!(
            offers[0]["terms"]["deliver_by"],
            json!(Date::from_days(20_000).iso())
        );
        assert_eq!(offers[0]["terms"]["payment_net_days"], json!(45));
        assert_eq!(offers[0]["round"], json!(1));
        assert_eq!(
            offers[1]["terms"]["move_in_by"],
            json!(Date::from_days(321).iso())
        );
    }
}

#[test]
fn kinds_says_dates_are_iso() {
    let v = block_json(&kinds());
    let date = v["purchase-order"]["fields"]["deliver_by"]
        .as_str()
        .unwrap();
    assert!(date.contains("YYYY-MM-DD"), "{date}");
    assert!(
        v["lease"]["fields"]["move_in_by"]
            .as_str()
            .unwrap()
            .contains("YYYY-MM-DD")
    );
    assert!(v["dates"].as_str().unwrap().contains("YYYY-MM-DD"));
}
