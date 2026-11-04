//! TermsHash: SHA-256 over canonical(terms, deal id, kind, parties); short() is 8 hex characters.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::deal_id::DealId;
use crate::error::DomainError;
use crate::party::ParticipantId;
use crate::terms::Terms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TermsHash([u8; 32]);

impl TermsHash {
    /// Hashes `Terms::canonical_bytes` (which starts with the kind), then `deal_id=<id>\n`,
    /// then `party_a=` and `party_b=` each as `<byte length>:<owner id>\n`. The length
    /// prefix keeps any owner id unambiguous.
    pub fn of(terms: &Terms, deal: &DealId, owners: [&ParticipantId; 2]) -> Self {
        Self(Sha256::digest(canonical(terms, deal, owners)).into())
    }

    pub fn short(&self) -> String {
        self.to_string()[..8].to_owned()
    }
}

fn canonical(terms: &Terms, deal: &DealId, [a, b]: [&ParticipantId; 2]) -> Vec<u8> {
    let mut out = terms.canonical_bytes();
    out.extend_from_slice(format!("deal_id={deal}\n").as_bytes());
    for (label, id) in [("party_a", a), ("party_b", b)] {
        let id = id.as_str();
        out.extend_from_slice(format!("{label}={}:{id}\n", id.len()).as_bytes());
    }
    out
}

impl fmt::Display for TermsHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

/// Accepts exactly 64 lowercase hex characters, the `Display` form.
impl FromStr for TermsHash {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, DomainError> {
        let valid = s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid {
            return Err(DomainError::InvalidHash);
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|_| DomainError::InvalidHash)?;
        }
        Ok(Self(out))
    }
}

impl TryFrom<String> for TermsHash {
    type Error = DomainError;
    fn try_from(s: String) -> Result<Self, DomainError> {
        s.parse()
    }
}

impl From<TermsHash> for String {
    fn from(h: TermsHash) -> String {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terms::fixtures::{lease, po};

    fn ids(a: &str, b: &str) -> [ParticipantId; 2] {
        [
            ParticipantId::new(a).unwrap(),
            ParticipantId::new(b).unwrap(),
        ]
    }

    fn hash(t: &Terms, deal: &str, owners: &[ParticipantId; 2]) -> TermsHash {
        TermsHash::of(t, &deal.parse().unwrap(), [&owners[0], &owners[1]])
    }

    #[test]
    fn hash_is_sha256_of_canonical_layout() {
        let t = po(109_000, 400, 100, 45);
        let o = ids("asha", "ravi");
        let mut bytes = t.canonical_bytes();
        bytes.extend_from_slice(b"deal_id=D-0007\nparty_a=4:asha\nparty_b=4:ravi\n");
        let expected: [u8; 32] = Sha256::digest(&bytes).into();
        assert_eq!(hash(&t, "D-0007", &o).0, expected);
    }

    #[test]
    fn hash_binds_terms_deal_kind_and_parties() {
        let t = po(109_000, 400, 100, 45);
        let o = ids("asha", "ravi");
        let h = hash(&t, "D-0007", &o);
        assert_eq!(h, hash(&po(109_000, 400, 100, 45), "D-0007", &o));
        assert_ne!(h, hash(&po(109_000, 400, 100, 30), "D-0007", &o));
        assert_ne!(h, hash(&lease(109_000, 4, 100, 45), "D-0007", &o));
        assert_ne!(h, hash(&t, "D-0009", &o));
        assert_ne!(h, hash(&t, "D-0007", &ids("ravi", "asha")));
        assert_ne!(
            hash(&t, "D-0007", &ids("a", "b:c")),
            hash(&t, "D-0007", &ids("a:b", "c"))
        );
    }

    #[test]
    fn hex_round_trip_and_short() {
        let h = hash(&lease(2_500_000, 2, 7, 11), "D-0009", &ids("asha", "ravi"));
        let hex = h.to_string();
        assert_eq!(hex.len(), 64);
        assert_eq!(h.short(), hex[..8]);
        assert_eq!(hex.parse::<TermsHash>().unwrap(), h);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<TermsHash>(&json).unwrap(), h);
        assert!(hex[..63].parse::<TermsHash>().is_err());
        assert!(hex.to_uppercase().parse::<TermsHash>().is_err());
        assert!("zz".repeat(32).parse::<TermsHash>().is_err());
    }
}
