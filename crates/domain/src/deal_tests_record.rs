//! Loading a stored deal re-checks its invariants.

use serde_json::{Value, json};

use super::super::fixtures::*;
use super::super::*;
use super::{A, B, agreed_po};
use crate::terms::fixtures::po;

fn bound() -> Deal {
    let mut d = agreed_po();
    let DealState::Agreed(h) = d.state() else {
        panic!()
    };
    sign(&mut d, A, h, owner(A), "m1").unwrap();
    sign(&mut d, B, h, owner(B), "m2").unwrap();
    d
}

fn load(v: Value) -> Result<Deal, serde_json::Error> {
    serde_json::from_value(v)
}

fn tampered(d: &Deal, f: impl FnOnce(&mut Value)) -> String {
    let mut v = serde_json::to_value(d).unwrap();
    f(&mut v);
    load(v).unwrap_err().to_string()
}

#[test]
fn valid_deals_round_trip_in_every_state() {
    let mut withdrawn = agreed_po();
    withdrawn.withdraw(&owner(A)).unwrap();
    let deals = [
        new_deal(DealKind::Lease, 8),
        ready(DealKind::PurchaseOrder, 8),
        agreed_po(),
        bound(),
        withdrawn,
    ];
    for d in deals {
        assert_eq!(load(serde_json::to_value(&d).unwrap()).unwrap(), d);
    }
}

#[test]
fn rejects_a_forged_agreed_hash() {
    let other = hash_of(&agreed_po(), &po(1, 400, 58, 45)).to_string();
    let e = tampered(&bound(), |v| v["state"] = json!({ "Bound": other }));
    assert!(e.contains("agreed hash"), "{e}");
}

#[test]
fn rejects_edited_agreed_terms() {
    let e = tampered(&bound(), |v| {
        v["offers"][0]["terms"]["price_per_kg"]["minor"] = json!(1);
    });
    assert!(e.contains("agreed hash"), "{e}");
}

#[test]
fn rejects_inconsistent_state_and_signatures() {
    let d = bound();
    let e = tampered(&d, |v| v["state"] = json!("Open"));
    assert!(e.contains("agreed hash"), "{e}");
    let e = tampered(&d, |v| {
        v["signatures"].as_array_mut().unwrap().pop();
    });
    assert!(e.contains("signature count"), "{e}");
    let e = tampered(&d, |v| v["signatures"][0]["signer"] = json!("owner-B"));
    assert!(e.contains("signer"), "{e}");
    let e = tampered(&d, |v| v["signatures"][1]["message_id"] = json!("m1"));
    assert!(e.contains("message ids"), "{e}");
}

#[test]
fn rejects_bad_parties_seals_and_offers() {
    let d = agreed_po();
    let e = tampered(&d, |v| v["bindings"][1]["owner"] = json!("owner-A"));
    assert!(e.contains("participants"), "{e}");
    let e = tampered(&d, |v| v["bindings"][0]["desk"] = json!("elsewhere"));
    assert!(e.contains("binding desk"), "{e}");
    let e = tampered(&d, |v| v["sealed"] = json!([true, false]));
    assert!(e.contains("seals"), "{e}");
    let e = tampered(&d, |v| v["opener"] = json!("B"));
    assert!(e.contains("offer turn"), "{e}");
    let e = tampered(&d, |v| v["offers"][0]["round"] = json!(2));
    assert!(e.contains("offer round"), "{e}");
    let e = tampered(&d, |v| v["bindings"][1] = Value::Null);
    assert!(e.contains("agreed without owners"), "{e}");
    let e = tampered(&d, |v| v["max_rounds"] = json!(0));
    assert!(e.contains("max_rounds"), "{e}");
    let e = tampered(&d, |v| v["extra"] = json!(1));
    assert!(e.contains("unknown field"), "{e}");
}
