//! Money { minor: i64, currency }: checked arithmetic, no floats.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Inr,
    Usd,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Inr => "INR",
            Currency::Usd => "USD",
        }
    }
}

/// Amount in the currency's minor unit (paise, cents).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    pub fn checked_add(self, other: Money) -> Result<Money, DomainError> {
        let minor = self.minor.checked_add(self.same(other)?.minor);
        minor
            .map(|m| Money::new(m, self.currency))
            .ok_or(DomainError::Overflow)
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, DomainError> {
        let minor = self.minor.checked_sub(self.same(other)?.minor);
        minor
            .map(|m| Money::new(m, self.currency))
            .ok_or(DomainError::Overflow)
    }

    pub fn checked_mul(self, factor: u32) -> Result<Money, DomainError> {
        self.minor
            .checked_mul(i64::from(factor))
            .map(|m| Money::new(m, self.currency))
            .ok_or(DomainError::Overflow)
    }

    fn same(self, other: Money) -> Result<Money, DomainError> {
        if self.currency == other.currency {
            Ok(other)
        } else {
            Err(DomainError::CurrencyMismatch)
        }
    }
}

/// Amounts in different currencies are unordered.
impl PartialOrd for Money {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self.currency == other.currency).then(|| self.minor.cmp(&other.minor))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.currency.code(), self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INR: Currency = Currency::Inr;

    #[test]
    fn adds_and_subtracts_same_currency() {
        let a = Money::new(109_000, INR);
        let b = Money::new(1_000, INR);
        assert_eq!(a.checked_add(b), Ok(Money::new(110_000, INR)));
        assert_eq!(a.checked_sub(b), Ok(Money::new(108_000, INR)));
        assert_eq!(b.checked_mul(400), Ok(Money::new(400_000, INR)));
    }

    #[test]
    fn rejects_mixed_currency_and_overflow() {
        let a = Money::new(1, INR);
        let b = Money::new(1, Currency::Usd);
        assert_eq!(a.checked_add(b), Err(DomainError::CurrencyMismatch));
        assert_eq!(a.checked_sub(b), Err(DomainError::CurrencyMismatch));
        let max = Money::new(i64::MAX, INR);
        assert_eq!(max.checked_add(a), Err(DomainError::Overflow));
        assert_eq!(
            Money::new(i64::MIN, INR).checked_sub(a),
            Err(DomainError::Overflow)
        );
        assert_eq!(max.checked_mul(2), Err(DomainError::Overflow));
    }

    #[test]
    fn orders_only_within_currency() {
        let a = Money::new(1, INR);
        assert!(a < Money::new(2, INR));
        assert_eq!(a.partial_cmp(&Money::new(1, Currency::Usd)), None);
    }

    #[test]
    fn serializes_currency_as_code() {
        let json = serde_json::to_string(&Money::new(5, INR)).unwrap();
        assert_eq!(json, r#"{"minor":5,"currency":"INR"}"#);
    }
}
