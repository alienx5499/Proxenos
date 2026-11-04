//! Fixtures: terms builders and distinctive sealed limits for both kinds.

use std::collections::BTreeMap;

use proxenos_domain::date::Date;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::limit::{Constraint, LimitSet};
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::party::Role;
use proxenos_domain::quantity::Quantity;
use proxenos_domain::terms::{FieldValue, LeaseTerms, PurchaseOrderTerms, Terms};

/// Every number sealed by `po_limits`; none may appear in any output.
pub const PO_SEALED: [i64; 10] = [115_037, 311, 493, 617, 31, 102_019, 207, 607, 503, 47];

fn inr(minor: i64) -> FieldValue {
    FieldValue::Money(Money::new(minor, Currency::Inr))
}

pub fn po(price: i64, kg: u32, day: i32, net: u16) -> Terms {
    let t = PurchaseOrderTerms::new(
        Money::new(price, Currency::Inr),
        Quantity::kg(kg),
        Date::from_days(day),
        net,
    );
    Terms::PurchaseOrder(t.unwrap())
}

pub fn lease(rent: i64, deposit: u8, day: i32, term: u8) -> Terms {
    Terms::Lease(LeaseTerms {
        monthly_rent: Money::new(rent, Currency::Inr),
        deposit_months: deposit,
        move_in_by: Date::from_days(day),
        term_months: term,
    })
}

pub fn limits(kind: DealKind, role: Role) -> LimitSet {
    use Constraint::*;
    let kg = |v| FieldValue::Quantity(Quantity::kg(v));
    let int = FieldValue::Integer;
    let map = match (kind, role) {
        (DealKind::PurchaseOrder, Role::A) => [
            (Field::PricePerKg, AtMost(inr(115_037))),
            (Field::QuantityKg, Between(kg(311), kg(493))),
            (Field::DeliverBy, NoLaterThan(Date::from_days(617))),
            (Field::PaymentNetDays, AtLeast(int(31))),
        ],
        (DealKind::PurchaseOrder, Role::B) => [
            (Field::PricePerKg, AtLeast(inr(102_019))),
            (Field::QuantityKg, Between(kg(207), kg(607))),
            (Field::DeliverBy, NoEarlierThan(Date::from_days(503))),
            (Field::PaymentNetDays, AtMost(int(47))),
        ],
        (DealKind::Lease, Role::A) => [
            (Field::MonthlyRent, AtMost(inr(3_000_000))),
            (Field::DepositMonths, AtMost(int(3))),
            (Field::MoveInBy, NoLaterThan(Date::from_days(40))),
            (Field::TermMonths, Between(int(6), int(12))),
        ],
        (DealKind::Lease, Role::B) => [
            (Field::MonthlyRent, AtLeast(inr(2_500_000))),
            (Field::DepositMonths, AtLeast(int(2))),
            (Field::MoveInBy, NoEarlierThan(Date::from_days(30))),
            (Field::TermMonths, Between(int(11), int(24))),
        ],
    };
    LimitSet::new(kind, role, BTreeMap::from(map)).unwrap()
}
