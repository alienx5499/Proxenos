//! DealKind::{PurchaseOrder, Lease}: field schema, constraint directions, role labels.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::party::Role;
use crate::quantity::Unit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DealKind {
    PurchaseOrder,
    Lease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    PricePerKg,
    QuantityKg,
    DeliverBy,
    PaymentNetDays,
    MonthlyRent,
    DepositMonths,
    MoveInBy,
    TermMonths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Money,
    Quantity(Unit),
    Date,
    Integer,
}

/// Which way a party's limit on a field points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    AtMost,
    AtLeast,
    NoLaterThan,
    NoEarlierThan,
    Between,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub field: Field,
    pub value_type: ValueType,
    pub role_a: Direction,
    pub role_b: Direction,
}

impl FieldSpec {
    pub fn direction(&self, role: Role) -> Direction {
        match role {
            Role::A => self.role_a,
            Role::B => self.role_b,
        }
    }
}

const fn spec(field: Field, value_type: ValueType, a: Direction, b: Direction) -> FieldSpec {
    FieldSpec {
        field,
        value_type,
        role_a: a,
        role_b: b,
    }
}

use Direction::*;

const PURCHASE_ORDER: [FieldSpec; 4] = [
    spec(Field::PricePerKg, ValueType::Money, AtMost, AtLeast),
    spec(
        Field::QuantityKg,
        ValueType::Quantity(Unit::Kg),
        Between,
        Between,
    ),
    spec(
        Field::DeliverBy,
        ValueType::Date,
        NoLaterThan,
        NoEarlierThan,
    ),
    spec(Field::PaymentNetDays, ValueType::Integer, AtLeast, AtMost),
];

const LEASE: [FieldSpec; 4] = [
    spec(Field::MonthlyRent, ValueType::Money, AtMost, AtLeast),
    spec(Field::DepositMonths, ValueType::Integer, AtMost, AtLeast),
    spec(Field::MoveInBy, ValueType::Date, NoLaterThan, NoEarlierThan),
    spec(Field::TermMonths, ValueType::Integer, Between, Between),
];

impl DealKind {
    pub fn name(self) -> &'static str {
        match self {
            DealKind::PurchaseOrder => "purchase-order",
            DealKind::Lease => "lease",
        }
    }

    /// Fields in canonical order.
    pub fn schema(self) -> &'static [FieldSpec] {
        match self {
            DealKind::PurchaseOrder => &PURCHASE_ORDER,
            DealKind::Lease => &LEASE,
        }
    }

    pub fn spec(self, field: Field) -> Option<&'static FieldSpec> {
        self.schema().iter().find(|s| s.field == field)
    }

    pub fn role_label(self, role: Role) -> &'static str {
        match (self, role) {
            (DealKind::PurchaseOrder, Role::A) => "Buyer",
            (DealKind::PurchaseOrder, Role::B) => "Seller",
            (DealKind::Lease, Role::A) => "Tenant",
            (DealKind::Lease, Role::B) => "Landlord",
        }
    }
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Field::PricePerKg => "price_per_kg",
            Field::QuantityKg => "quantity_kg",
            Field::DeliverBy => "deliver_by",
            Field::PaymentNetDays => "payment_net_days",
            Field::MonthlyRent => "monthly_rent",
            Field::DepositMonths => "deposit_months",
            Field::MoveInBy => "move_in_by",
            Field::TermMonths => "term_months",
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchase_order_directions_match_design() {
        let k = DealKind::PurchaseOrder;
        let dir = |r, f| k.spec(f).unwrap().direction(r);
        assert_eq!(dir(Role::A, Field::PricePerKg), AtMost);
        assert_eq!(dir(Role::B, Field::PricePerKg), AtLeast);
        assert_eq!(dir(Role::A, Field::QuantityKg), Between);
        assert_eq!(dir(Role::B, Field::QuantityKg), Between);
        assert_eq!(dir(Role::A, Field::DeliverBy), NoLaterThan);
        assert_eq!(dir(Role::B, Field::DeliverBy), NoEarlierThan);
        assert_eq!(dir(Role::A, Field::PaymentNetDays), AtLeast);
        assert_eq!(dir(Role::B, Field::PaymentNetDays), AtMost);
        assert!(k.spec(Field::MonthlyRent).is_none());
    }

    #[test]
    fn lease_directions_match_design() {
        let k = DealKind::Lease;
        let dir = |r, f| k.spec(f).unwrap().direction(r);
        assert_eq!(dir(Role::A, Field::MonthlyRent), AtMost);
        assert_eq!(dir(Role::B, Field::MonthlyRent), AtLeast);
        assert_eq!(dir(Role::A, Field::DepositMonths), AtMost);
        assert_eq!(dir(Role::B, Field::DepositMonths), AtLeast);
        assert_eq!(dir(Role::A, Field::MoveInBy), NoLaterThan);
        assert_eq!(dir(Role::B, Field::MoveInBy), NoEarlierThan);
        assert_eq!(dir(Role::A, Field::TermMonths), Between);
        assert_eq!(dir(Role::B, Field::TermMonths), Between);
        assert!(k.spec(Field::PricePerKg).is_none());
    }

    #[test]
    fn role_labels_per_kind() {
        assert_eq!(DealKind::PurchaseOrder.role_label(Role::A), "Buyer");
        assert_eq!(DealKind::PurchaseOrder.role_label(Role::B), "Seller");
        assert_eq!(DealKind::Lease.role_label(Role::A), "Tenant");
        assert_eq!(DealKind::Lease.role_label(Role::B), "Landlord");
    }

    #[test]
    fn kind_names_match_serde() {
        for k in [DealKind::PurchaseOrder, DealKind::Lease] {
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.name()));
        }
    }

    #[test]
    fn field_names_match_serde() {
        for k in [DealKind::PurchaseOrder, DealKind::Lease] {
            for s in k.schema() {
                let json = serde_json::to_string(&s.field).unwrap();
                assert_eq!(json, format!("\"{}\"", s.field.name()));
            }
        }
    }
}
