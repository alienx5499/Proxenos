//! DealId: the operator's `D-dddd`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DealId(String);

impl DealId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DealId {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, DomainError> {
        match s.strip_prefix("D-") {
            Some(d) if d.len() == 4 && d.bytes().all(|b| b.is_ascii_digit()) => Ok(Self(s.into())),
            _ => Err(DomainError::InvalidDealId),
        }
    }
}

impl TryFrom<String> for DealId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, DomainError> {
        s.parse()
    }
}

impl From<DealId> for String {
    fn from(id: DealId) -> String {
        id.0
    }
}

impl fmt::Display for DealId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_d_dddd() {
        assert_eq!("D-0007".parse::<DealId>().unwrap().as_str(), "D-0007");
        for bad in ["D-007", "D-00071", "d-0007", "D-00a7", "0007", "D-+007"] {
            assert!(bad.parse::<DealId>().is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<DealId>("\"X-0001\"").is_err());
    }
}
