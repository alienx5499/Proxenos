use super::*;
use crate::money::{Currency, Money};
use crate::quantity::{Quantity, Unit};
use crate::terms::PurchaseOrderTerms;
use crate::terms::fixtures::{lease, po};
use proptest::prelude::*;

fn inr(minor: i64) -> FieldValue {
    FieldValue::Money(Money::new(minor, Currency::Inr))
}

fn kg(v: u32) -> FieldValue {
    FieldValue::Quantity(Quantity::kg(v))
}

fn int(n: u32) -> FieldValue {
    FieldValue::Integer(n)
}

fn buyer(price: i64, lo: u32, hi: u32, day: i32, net: u32) -> Result<LimitSet, DomainError> {
    let limits = BTreeMap::from([
        (Field::PricePerKg, Constraint::AtMost(inr(price))),
        (Field::QuantityKg, Constraint::Between(kg(lo), kg(hi))),
        (
            Field::DeliverBy,
            Constraint::NoLaterThan(Date::from_days(day)),
        ),
        (Field::PaymentNetDays, Constraint::AtLeast(int(net))),
    ]);
    LimitSet::new(DealKind::PurchaseOrder, Role::A, limits)
}

fn seller(price: i64, lo: u32, hi: u32, day: i32, net: u32) -> LimitSet {
    let limits = BTreeMap::from([
        (Field::PricePerKg, Constraint::AtLeast(inr(price))),
        (Field::QuantityKg, Constraint::Between(kg(lo), kg(hi))),
        (
            Field::DeliverBy,
            Constraint::NoEarlierThan(Date::from_days(day)),
        ),
        (Field::PaymentNetDays, Constraint::AtMost(int(net))),
    ]);
    LimitSet::new(DealKind::PurchaseOrder, Role::B, limits).unwrap()
}

#[test]
fn design_example_counter_fails_only_payment() {
    let s = seller(102_000, 200, 600, 50, 45);
    let v = s.verdict(&po(109_000, 400, 58, 60));
    assert_eq!(
        v,
        OwnVerdict {
            within: false,
            failing_fields: vec![Field::PaymentNetDays]
        }
    );
    assert!(s.verdict(&po(109_000, 400, 58, 45)).within);
}

#[test]
fn bounds_are_inclusive() {
    let b = buyer(115_000, 300, 500, 60, 30).unwrap();
    assert!(b.verdict(&po(115_000, 300, 60, 30)).within);
    assert!(b.verdict(&po(115_000, 500, 60, 30)).within);
    let v = b.verdict(&po(115_001, 501, 61, 29));
    assert_eq!(v.failing_fields.len(), 4);
}

#[test]
fn other_kind_or_currency_fails() {
    let b = buyer(115_000, 300, 500, 60, 30).unwrap();
    assert_eq!(b.verdict(&lease(1, 1, 1, 1)).failing_fields.len(), 4);
    let usd = Money::new(1, Currency::Usd);
    let t = PurchaseOrderTerms::new(usd, Quantity::kg(400), Date::from_days(1), 30).unwrap();
    let v = b.verdict(&Terms::PurchaseOrder(t));
    assert_eq!(v.failing_fields, vec![Field::PricePerKg]);
}

#[test]
fn rejects_wrong_direction_type_range_and_coverage() {
    let wrong_dir = BTreeMap::from([(Field::PricePerKg, Constraint::AtLeast(inr(1)))]);
    let mut limits = buyer(1, 1, 2, 1, 1).unwrap().limits;
    limits.extend(wrong_dir);
    let err = LimitSet::new(DealKind::PurchaseOrder, Role::A, limits.clone());
    assert_eq!(err, Err(DomainError::WrongDirection(Field::PricePerKg)));

    limits.insert(Field::PricePerKg, Constraint::AtMost(int(1)));
    let err = LimitSet::new(DealKind::PurchaseOrder, Role::A, limits.clone());
    assert_eq!(err, Err(DomainError::WrongValueType(Field::PricePerKg)));

    let months = FieldValue::Quantity(Quantity::new(1, Unit::Months));
    limits.insert(Field::PricePerKg, Constraint::AtMost(inr(1)));
    limits.insert(Field::QuantityKg, Constraint::Between(months, months));
    let err = LimitSet::new(DealKind::PurchaseOrder, Role::A, limits.clone());
    assert_eq!(err, Err(DomainError::WrongValueType(Field::QuantityKg)));

    assert_eq!(
        buyer(1, 5, 4, 1, 1),
        Err(DomainError::EmptyRange(Field::QuantityKg))
    );

    limits.remove(&Field::QuantityKg);
    let err = LimitSet::new(DealKind::PurchaseOrder, Role::A, limits.clone());
    assert_eq!(err, Err(DomainError::MissingLimit(Field::QuantityKg)));

    limits.insert(Field::TermMonths, Constraint::Between(int(1), int(2)));
    let err = LimitSet::new(DealKind::PurchaseOrder, Role::A, limits);
    assert_eq!(err, Err(DomainError::UnexpectedLimit(Field::TermMonths)));
}

#[test]
fn lease_limits_validate_and_judge() {
    let tenant = LimitSet::new(
        DealKind::Lease,
        Role::A,
        BTreeMap::from([
            (Field::MonthlyRent, Constraint::AtMost(inr(3_000_000))),
            (Field::DepositMonths, Constraint::AtMost(int(3))),
            (
                Field::MoveInBy,
                Constraint::NoLaterThan(Date::from_days(40)),
            ),
            (Field::TermMonths, Constraint::Between(int(11), int(24))),
        ]),
    )
    .unwrap();
    assert!(tenant.verdict(&lease(2_500_000, 2, 30, 11)).within);
    let v = tenant.verdict(&lease(2_500_000, 4, 30, 36));
    assert_eq!(
        v.failing_fields,
        vec![Field::DepositMonths, Field::TermMonths]
    );
}

#[test]
fn money_limits_share_one_currency() {
    let usd = FieldValue::Money(Money::new(5, Currency::Usd));
    let mut limits = buyer(1, 1, 2, 1, 1).unwrap().limits;
    assert!(single_currency(&limits));
    limits.insert(Field::PricePerKg, Constraint::AtMost(usd));
    assert!(single_currency(&limits));
    limits.insert(Field::MonthlyRent, Constraint::AtMost(inr(1)));
    assert!(!single_currency(&limits));
    assert!(!single_currency(&BTreeMap::from([(
        Field::PricePerKg,
        Constraint::Between(inr(1), usd)
    )])));
}

#[test]
fn landlord_verdict_fails_low_rent_deposit_and_early_move_in() {
    let landlord = LimitSet::new(
        DealKind::Lease,
        Role::B,
        BTreeMap::from([
            (Field::MonthlyRent, Constraint::AtLeast(inr(2_000_000))),
            (Field::DepositMonths, Constraint::AtLeast(int(2))),
            (
                Field::MoveInBy,
                Constraint::NoEarlierThan(Date::from_days(20)),
            ),
            (Field::TermMonths, Constraint::Between(int(11), int(36))),
        ]),
    )
    .unwrap();
    assert!(landlord.verdict(&lease(2_000_000, 2, 20, 11)).within);
    let v = landlord.verdict(&lease(1_999_999, 1, 19, 37));
    assert_eq!(
        v.failing_fields,
        vec![
            Field::MonthlyRent,
            Field::DepositMonths,
            Field::MoveInBy,
            Field::TermMonths
        ]
    );
    let v = landlord.verdict(&lease(2_500_000, 3, 10, 12));
    assert_eq!(v.failing_fields, vec![Field::MoveInBy]);
}

#[test]
fn deserialize_validates() {
    let b = buyer(115_000, 300, 500, 60, 30).unwrap();
    let json = serde_json::to_string(&b).unwrap();
    assert_eq!(serde_json::from_str::<LimitSet>(&json).unwrap(), b);
    let bad = json.replace("at_most", "at_least");
    assert!(serde_json::from_str::<LimitSet>(&bad).is_err());
}

proptest! {
    #[test]
    fn verdict_matches_raw_comparison(
        max in 0i64..2_000_000, a in 0u32..1000, b in 0u32..1000,
        day in -500i32..500, net in 0u32..120,
        price in 0i64..2_000_000, kgs in 0u32..1000, deliver in -500i32..500, pay in 0u16..120,
    ) {
        let (lo, hi) = (a.min(b), a.max(b));
        let limits = buyer(max, lo, hi, day, net).unwrap();
        let v = limits.verdict(&po(price, kgs, deliver, pay));
        let mut expected = Vec::new();
        if price > max { expected.push(Field::PricePerKg); }
        if kgs < lo || kgs > hi { expected.push(Field::QuantityKg); }
        if deliver > day { expected.push(Field::DeliverBy); }
        if u32::from(pay) < net { expected.push(Field::PaymentNetDays); }
        prop_assert_eq!(v.within, expected.is_empty());
        prop_assert_eq!(v.failing_fields, expected);
    }

    #[test]
    fn seller_verdict_mirrors_buyer(
        min in 0i64..2_000_000, price in 0i64..2_000_000, net in 0u32..120, pay in 0u16..120,
        min_day in -500i32..500, deliver in -500i32..500,
    ) {
        let s = seller(min, 0, u32::MAX, min_day, net);
        let v = s.verdict(&po(price, 10, deliver, pay));
        prop_assert_eq!(v.failing_fields.contains(&Field::PricePerKg), price < min);
        prop_assert_eq!(v.failing_fields.contains(&Field::DeliverBy), deliver < min_day);
        prop_assert_eq!(v.failing_fields.contains(&Field::PaymentNetDays), u32::from(pay) > net);
    }
}
