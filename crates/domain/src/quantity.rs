//! Quantity { value: u32, unit }: kg, months.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Kg,
    Months,
}

impl Unit {
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Kg => "kg",
            Unit::Months => "months",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Quantity {
    pub value: u32,
    pub unit: Unit,
}

impl Quantity {
    pub fn new(value: u32, unit: Unit) -> Self {
        Self { value, unit }
    }

    pub fn kg(value: u32) -> Self {
        Self::new(value, Unit::Kg)
    }
}

/// Quantities in different units are unordered.
impl PartialOrd for Quantity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self.unit == other.unit).then(|| self.value.cmp(&other.value))
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.value, self.unit.symbol())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_only_within_unit() {
        assert!(Quantity::kg(300) < Quantity::kg(400));
        let months = Quantity::new(300, Unit::Months);
        assert_eq!(Quantity::kg(300).partial_cmp(&months), None);
    }

    #[test]
    fn displays_value_and_unit() {
        assert_eq!(Quantity::kg(400).to_string(), "400 kg");
        assert_eq!(Quantity::new(11, Unit::Months).to_string(), "11 months");
    }
}
