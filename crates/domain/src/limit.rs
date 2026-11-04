//! Constraint and LimitSet::verdict(&Terms) -> OwnVerdict.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::date::Date;
use crate::error::DomainError;
use crate::kind::{DealKind, Direction, Field};
use crate::party::Role;
use crate::terms::{FieldValue, Terms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    AtMost(FieldValue),
    AtLeast(FieldValue),
    NoLaterThan(Date),
    NoEarlierThan(Date),
    /// Inclusive on both ends.
    Between(FieldValue, FieldValue),
}

impl Constraint {
    pub fn direction(self) -> Direction {
        match self {
            Constraint::AtMost(_) => Direction::AtMost,
            Constraint::AtLeast(_) => Direction::AtLeast,
            Constraint::NoLaterThan(_) => Direction::NoLaterThan,
            Constraint::NoEarlierThan(_) => Direction::NoEarlierThan,
            Constraint::Between(..) => Direction::Between,
        }
    }

    fn bounds(self) -> Vec<FieldValue> {
        match self {
            Constraint::AtMost(v) | Constraint::AtLeast(v) => vec![v],
            Constraint::NoLaterThan(d) | Constraint::NoEarlierThan(d) => vec![FieldValue::Date(d)],
            Constraint::Between(lo, hi) => vec![lo, hi],
        }
    }

    /// Unordered values (other type, currency or unit) never satisfy a constraint.
    pub fn admits(self, value: FieldValue) -> bool {
        use Ordering::*;
        let cmp = |bound: FieldValue| value.partial_cmp(&bound);
        match self {
            Constraint::AtMost(b) => matches!(cmp(b), Some(Less | Equal)),
            Constraint::AtLeast(b) => matches!(cmp(b), Some(Greater | Equal)),
            Constraint::NoLaterThan(d) => matches!(cmp(FieldValue::Date(d)), Some(Less | Equal)),
            Constraint::NoEarlierThan(d) => {
                matches!(cmp(FieldValue::Date(d)), Some(Greater | Equal))
            }
            Constraint::Between(lo, hi) => {
                matches!(cmp(lo), Some(Greater | Equal)) && matches!(cmp(hi), Some(Less | Equal))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnVerdict {
    pub within: bool,
    pub failing_fields: Vec<Field>,
}

/// One party's sealed limits: exactly one constraint per field of the kind,
/// each pointing the way the kind says for that role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawLimitSet")]
pub struct LimitSet {
    kind: DealKind,
    role: Role,
    limits: BTreeMap<Field, Constraint>,
}

#[derive(Deserialize)]
struct RawLimitSet {
    kind: DealKind,
    role: Role,
    limits: BTreeMap<Field, Constraint>,
}

impl TryFrom<RawLimitSet> for LimitSet {
    type Error = DomainError;
    fn try_from(raw: RawLimitSet) -> Result<Self, DomainError> {
        LimitSet::new(raw.kind, raw.role, raw.limits)
    }
}

impl LimitSet {
    pub fn new(
        kind: DealKind,
        role: Role,
        limits: BTreeMap<Field, Constraint>,
    ) -> Result<Self, DomainError> {
        if let Some(extra) = limits.keys().find(|f| kind.spec(**f).is_none()) {
            return Err(DomainError::UnexpectedLimit(*extra));
        }
        for spec in kind.schema() {
            let field = spec.field;
            let c = limits.get(&field).ok_or(DomainError::MissingLimit(field))?;
            if c.direction() != spec.direction(role) {
                return Err(DomainError::WrongDirection(field));
            }
            if c.bounds().iter().any(|b| b.value_type() != spec.value_type) {
                return Err(DomainError::WrongValueType(field));
            }
            if let Constraint::Between(lo, hi) = c
                && lo.partial_cmp(hi) == Some(Ordering::Greater)
            {
                return Err(DomainError::EmptyRange(field));
            }
        }
        if !single_currency(&limits) {
            return Err(DomainError::CurrencyMismatch);
        }
        Ok(Self { kind, role, limits })
    }

    pub fn kind(&self) -> DealKind {
        self.kind
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn get(&self, field: Field) -> Option<Constraint> {
        self.limits.get(&field).copied()
    }

    /// `failing_fields` lists fields in the kind's schema order; callers may rely on it.
    /// Every field fails when the terms are of another kind; a value in another
    /// currency or unit fails its field.
    pub fn verdict(&self, terms: &Terms) -> OwnVerdict {
        let failing_fields: Vec<Field> = self
            .kind
            .schema()
            .iter()
            .map(|s| s.field)
            .filter(|f| {
                let c = self.limits.get(f).expect("LimitSet covers every field");
                !terms.value(*f).is_some_and(|v| c.admits(v))
            })
            .collect();
        OwnVerdict {
            within: failing_fields.is_empty(),
            failing_fields,
        }
    }
}

/// All Money bounds in one set share a currency. Each current kind has one Money
/// field with a one-sided limit, so this guards schemas that add more.
fn single_currency(limits: &BTreeMap<Field, Constraint>) -> bool {
    let mut currencies = limits
        .values()
        .flat_map(|c| c.bounds())
        .filter_map(|b| match b {
            FieldValue::Money(m) => Some(m.currency),
            _ => None,
        });
    match currencies.next() {
        Some(first) => currencies.all(|c| c == first),
        None => true,
    }
}

#[cfg(test)]
#[path = "limit_tests.rs"]
mod tests;
