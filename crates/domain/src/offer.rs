//! Offer { id, by, round, terms, note }; Note is stripped of '@', then at most 280 chars.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::party::Role;
use crate::signature::MessageId;
use crate::terms::Terms;

/// `O-<n>`, numbered from 1 in the order offers are recorded on a deal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OfferId(u32);

impl OfferId {
    pub fn new(n: u32) -> Result<Self, DomainError> {
        if n == 0 {
            return Err(DomainError::InvalidOfferId);
        }
        Ok(Self(n))
    }

    pub fn number(self) -> u32 {
        self.0
    }
}

impl FromStr for OfferId {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, DomainError> {
        let digits = s.strip_prefix("O-").ok_or(DomainError::InvalidOfferId)?;
        let digits_only = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
        if !digits_only || digits.starts_with('0') {
            return Err(DomainError::InvalidOfferId);
        }
        Self::new(digits.parse().map_err(|_| DomainError::InvalidOfferId)?)
    }
}

impl TryFrom<String> for OfferId {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, DomainError> {
        s.parse()
    }
}

impl From<OfferId> for String {
    fn from(id: OfferId) -> String {
        id.to_string()
    }
}

impl fmt::Display for OfferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "O-{}", self.0)
    }
}

pub const NOTE_MAX_CHARS: usize = 280;

/// Counterparty-visible free text; never part of binding terms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Note(String);

impl Note {
    /// Removes every '@' so a note can never address anyone, then checks the length.
    pub fn new(text: impl Into<String>) -> Result<Self, DomainError> {
        let text = text.into().replace('@', "");
        if text.chars().count() > NOTE_MAX_CHARS {
            return Err(DomainError::NoteTooLong);
        }
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Note {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, DomainError> {
        Self::new(s)
    }
}

impl From<Note> for String {
    fn from(n: Note) -> String {
        n.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: OfferId,
    pub by: Role,
    pub round: u16,
    pub terms: Terms,
    pub note: Note,
    pub message_id: MessageId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_limits() {
        assert!(Note::new("a".repeat(280)).is_ok());
        assert_eq!(Note::new("é".repeat(281)), Err(DomainError::NoteTooLong));
        assert_eq!(Note::new("hi @seller").unwrap().as_str(), "hi seller");
        let n: Note = serde_json::from_str("\"ping @@x\"").unwrap();
        assert_eq!(n.as_str(), "ping x");
        let padded = format!("{}{}", "a".repeat(280), "@".repeat(5));
        assert_eq!(Note::new(padded).unwrap().as_str().len(), 280);
        assert_eq!(Note::default().as_str(), "");
    }

    #[test]
    fn offer_id_round_trips() {
        let id: OfferId = "O-5".parse().unwrap();
        assert_eq!(id.number(), 5);
        assert_eq!(id.to_string(), "O-5");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"O-5\"");
        for bad in ["O-0", "O-05", "O-", "5", "O-+5", "o-5", "O-5x"] {
            assert!(bad.parse::<OfferId>().is_err(), "{bad}");
        }
    }
}
