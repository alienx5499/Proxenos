//! Terms per deal kind; canonical_bytes() with a fixed field order.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::date::Date;
use crate::error::DomainError;
use crate::kind::{DealKind, Field, ValueType};
use crate::money::Money;
use crate::quantity::{Quantity, Unit};

/// Built only through `new` or deserialization, both of which check the quantity unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawPurchaseOrderTerms")]
pub struct PurchaseOrderTerms {
    price_per_kg: Money,
    quantity_kg: Quantity,
    deliver_by: Date,
    payment_net_days: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPurchaseOrderTerms {
    price_per_kg: Money,
    quantity_kg: Quantity,
    deliver_by: Date,
    payment_net_days: u16,
}

impl TryFrom<RawPurchaseOrderTerms> for PurchaseOrderTerms {
    type Error = DomainError;
    fn try_from(r: RawPurchaseOrderTerms) -> Result<Self, DomainError> {
        Self::new(
            r.price_per_kg,
            r.quantity_kg,
            r.deliver_by,
            r.payment_net_days,
        )
    }
}

impl PurchaseOrderTerms {
    pub fn new(
        price_per_kg: Money,
        quantity_kg: Quantity,
        deliver_by: Date,
        payment_net_days: u16,
    ) -> Result<Self, DomainError> {
        if quantity_kg.unit != Unit::Kg {
            return Err(DomainError::WrongValueType(Field::QuantityKg));
        }
        Ok(Self {
            price_per_kg,
            quantity_kg,
            deliver_by,
            payment_net_days,
        })
    }

    pub fn price_per_kg(&self) -> Money {
        self.price_per_kg
    }

    pub fn quantity_kg(&self) -> Quantity {
        self.quantity_kg
    }

    pub fn deliver_by(&self) -> Date {
        self.deliver_by
    }

    pub fn payment_net_days(&self) -> u16 {
        self.payment_net_days
    }
}

/// Every combination of these field types is valid, so the fields stay public.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseTerms {
    pub monthly_rent: Money,
    pub deposit_months: u8,
    pub move_in_by: Date,
    pub term_months: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Terms {
    PurchaseOrder(PurchaseOrderTerms),
    Lease(LeaseTerms),
}

/// One field's value, typed for comparison against a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldValue {
    Money(Money),
    Quantity(Quantity),
    Date(Date),
    Integer(u32),
}

impl FieldValue {
    pub fn value_type(self) -> ValueType {
        match self {
            FieldValue::Money(_) => ValueType::Money,
            FieldValue::Quantity(q) => ValueType::Quantity(q.unit),
            FieldValue::Date(_) => ValueType::Date,
            FieldValue::Integer(_) => ValueType::Integer,
        }
    }
}

/// Values of different types, currencies or units are unordered.
impl PartialOrd for FieldValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (FieldValue::Money(a), FieldValue::Money(b)) => a.partial_cmp(b),
            (FieldValue::Quantity(a), FieldValue::Quantity(b)) => a.partial_cmp(b),
            (FieldValue::Date(a), FieldValue::Date(b)) => a.partial_cmp(b),
            (FieldValue::Integer(a), FieldValue::Integer(b)) => a.partial_cmp(b),
            _ => None,
        }
    }
}

/// Canonical text form; part of the hashed bytes, so changing it changes every hash.
impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldValue::Money(m) => write!(f, "{} {}", m.currency.code(), m.minor),
            FieldValue::Quantity(q) => write!(f, "{} {}", q.value, q.unit.symbol()),
            FieldValue::Date(d) => write!(f, "{}", d.days()),
            FieldValue::Integer(n) => write!(f, "{n}"),
        }
    }
}

impl Terms {
    pub fn kind(&self) -> DealKind {
        match self {
            Terms::PurchaseOrder(_) => DealKind::PurchaseOrder,
            Terms::Lease(_) => DealKind::Lease,
        }
    }

    /// None when the field does not belong to this kind.
    pub fn value(&self, field: Field) -> Option<FieldValue> {
        use FieldValue as V;
        Some(match (self, field) {
            (Terms::PurchaseOrder(t), Field::PricePerKg) => V::Money(t.price_per_kg),
            (Terms::PurchaseOrder(t), Field::QuantityKg) => V::Quantity(t.quantity_kg),
            (Terms::PurchaseOrder(t), Field::DeliverBy) => V::Date(t.deliver_by),
            (Terms::PurchaseOrder(t), Field::PaymentNetDays) => {
                V::Integer(t.payment_net_days.into())
            }
            (Terms::Lease(t), Field::MonthlyRent) => V::Money(t.monthly_rent),
            (Terms::Lease(t), Field::DepositMonths) => V::Integer(t.deposit_months.into()),
            (Terms::Lease(t), Field::MoveInBy) => V::Date(t.move_in_by),
            (Terms::Lease(t), Field::TermMonths) => V::Integer(t.term_months.into()),
            _ => return None,
        })
    }

    /// True when any date field falls before `today`.
    pub fn has_date_before(&self, today: Date) -> bool {
        self.kind()
            .schema()
            .iter()
            .any(|s| matches!(self.value(s.field), Some(FieldValue::Date(d)) if d < today))
    }

    /// `kind=<kind>\n` then `<field>=<value>\n` per field in schema order.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let kind = self.kind();
        let mut out = format!("kind={}\n", kind.name());
        for spec in kind.schema() {
            if let Some(v) = self.value(spec.field) {
                out.push_str(&format!("{}={v}\n", spec.field.name()));
            }
        }
        out.into_bytes()
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::money::Currency;

    pub fn po(price: i64, kg: u32, day: i32, net: u16) -> Terms {
        let price = Money::new(price, Currency::Inr);
        let t = PurchaseOrderTerms::new(price, Quantity::kg(kg), Date::from_days(day), net);
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
}

#[cfg(test)]
#[path = "terms_tests.rs"]
mod tests;
