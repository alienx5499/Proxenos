use super::fixtures::{lease, po};
use super::*;
use crate::quantity::Unit;
use proptest::prelude::*;

#[test]
fn canonical_bytes_has_fixed_layout() {
    let bytes = po(109_000, 400, 100, 45).canonical_bytes();
    let expected = "kind=purchase-order\nprice_per_kg=INR 109000\nquantity_kg=400 kg\ndeliver_by=100\npayment_net_days=45\n";
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
    let bytes = lease(2_500_000, 2, 7, 11).canonical_bytes();
    let expected =
        "kind=lease\nmonthly_rent=INR 2500000\ndeposit_months=2\nmove_in_by=7\nterm_months=11\n";
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[test]
fn canonical_bytes_ignore_json_key_order() {
    let a = r#"{"kind":"purchase-order","price_per_kg":{"minor":5,"currency":"INR"},"quantity_kg":{"value":3,"unit":"kg"},"deliver_by":9,"payment_net_days":30}"#;
    let b = r#"{"payment_net_days":30,"deliver_by":9,"quantity_kg":{"unit":"kg","value":3},"price_per_kg":{"currency":"INR","minor":5},"kind":"purchase-order"}"#;
    let a: Terms = serde_json::from_str(a).unwrap();
    let b: Terms = serde_json::from_str(b).unwrap();
    assert_eq!(a.canonical_bytes(), b.canonical_bytes());
}

#[test]
fn rejects_unknown_json_fields() {
    let j = r#"{"kind":"lease","monthly_rent":{"minor":5,"currency":"INR"},"deposit_months":1,"move_in_by":2,"term_months":3,"extra":1}"#;
    assert!(serde_json::from_str::<Terms>(j).is_err());
}

#[test]
fn rejects_wrong_unit_on_construction_and_deserialize() {
    let price = Money::new(1, crate::money::Currency::Inr);
    let months = Quantity::new(1, Unit::Months);
    let err = PurchaseOrderTerms::new(price, months, Date::from_days(1), 1);
    assert_eq!(err, Err(DomainError::WrongValueType(Field::QuantityKg)));
    let j = r#"{"kind":"purchase-order","price_per_kg":{"minor":5,"currency":"INR"},"quantity_kg":{"value":3,"unit":"months"},"deliver_by":9,"payment_net_days":30}"#;
    assert!(serde_json::from_str::<Terms>(j).is_err());
    let snake = j
        .replace("purchase-order", "purchase_order")
        .replace("months", "kg");
    assert!(serde_json::from_str::<Terms>(&snake).is_err());
    assert!(
        serde_json::from_str::<Terms>(&snake.replace("purchase_order", "purchase-order")).is_ok()
    );
}

#[test]
fn rejects_unknown_purchase_order_field() {
    let j = r#"{"kind":"purchase-order","price_per_kg":{"minor":5,"currency":"INR"},"quantity_kg":{"value":3,"unit":"kg"},"deliver_by":9,"payment_net_days":30,"x":1}"#;
    assert!(serde_json::from_str::<Terms>(j).is_err());
}

#[test]
fn value_is_none_for_foreign_field() {
    assert_eq!(po(1, 1, 1, 1).value(Field::MonthlyRent), None);
    assert_eq!(lease(1, 1, 1, 1).value(Field::PricePerKg), None);
}

fn any_terms() -> impl Strategy<Value = Terms> {
    prop_oneof![
        (any::<i64>(), any::<u32>(), any::<i32>(), any::<u16>())
            .prop_map(|(p, q, d, n)| po(p, q, d, n)),
        (any::<i64>(), any::<u8>(), any::<i32>(), any::<u8>())
            .prop_map(|(r, dep, d, t)| lease(r, dep, d, t)),
    ]
}

#[test]
fn has_date_before_compares_each_date_field() {
    let today = Date::from_days(500);
    assert!(po(100, 1, 499, 30).has_date_before(today));
    assert!(!po(100, 1, 500, 30).has_date_before(today));
    assert!(lease(100, 1, 10, 11).has_date_before(today));
    assert!(!lease(100, 1, 900, 11).has_date_before(today));
}

proptest! {
    #[test]
    fn canonical_bytes_stable_across_serde(t in any_terms()) {
        let back: Terms = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        prop_assert_eq!(t.canonical_bytes(), back.canonical_bytes());
        prop_assert_eq!(t.canonical_bytes(), t.clone().canonical_bytes());
    }

    #[test]
    fn canonical_bytes_injective(a in any_terms(), b in any_terms()) {
        prop_assert_eq!(a == b, a.canonical_bytes() == b.canonical_bytes());
    }

    #[test]
    fn canonical_bytes_differ_on_one_field_change(t in any_terms(), bump in 1i64..1000) {
        let changed = match &t {
            Terms::PurchaseOrder(p) => {
                let price = p.price_per_kg().minor.wrapping_add(bump);
                let net = p.payment_net_days();
                po(price, p.quantity_kg().value, p.deliver_by().days(), net)
            }
            Terms::Lease(l) => {
                let mut l = l.clone();
                l.move_in_by = Date::from_days(l.move_in_by.days().wrapping_add(bump as i32));
                Terms::Lease(l)
            }
        };
        prop_assert_ne!(t.canonical_bytes(), changed.canonical_bytes());
    }
}
