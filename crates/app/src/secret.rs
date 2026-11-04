//! Seal tokens, join codes, and SecretHash (HMAC-SHA256 under a server key); only hashes are stored.

use std::fmt;
use std::str::FromStr;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretHash([u8; 32]);

impl SecretHash {
    pub fn keyed(key: &[u8], secret: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
        mac.update(secret.as_bytes());
        Self(mac.finalize().into_bytes().into())
    }
}

/// A seal token: 256 random bits as hex.
pub fn seal_token(random: [u8; 32]) -> String {
    hex(&random)
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A join code: 40 random bits as 8 Crockford base32 characters, shown `XXXX-XXXX`.
pub fn join_code(random: [u8; 32]) -> String {
    let bits = random[..5]
        .iter()
        .fold(0u64, |acc, b| acc << 8 | u64::from(*b));
    let chars: String = (0..8)
        .rev()
        .map(|i| char::from(CROCKFORD[((bits >> (i * 5)) & 31) as usize]))
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// The form a join code is hashed in, at issue and at bind: no dashes or spaces,
/// uppercase, and Crockford's look-alikes folded (I, L to 1; O to 0).
pub fn normalize_join_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        })
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex to bytes.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let lower = s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !lower || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Exactly `2 * N` hex characters, either case, surrounding whitespace ignored.
pub fn unhex_array<const N: usize>(s: &str) -> Option<[u8; N]> {
    unhex(&s.trim().to_ascii_lowercase())?.try_into().ok()
}

impl fmt::Display for SecretHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("secret hash must be 64 lowercase hex characters")]
pub struct InvalidSecretHash;

impl FromStr for SecretHash {
    type Err = InvalidSecretHash;
    fn from_str(s: &str) -> Result<Self, InvalidSecretHash> {
        let bytes = unhex(s).ok_or(InvalidSecretHash)?;
        Ok(Self(bytes.try_into().map_err(|_| InvalidSecretHash)?))
    }
}

impl TryFrom<String> for SecretHash {
    type Error = InvalidSecretHash;
    fn try_from(s: String) -> Result<Self, InvalidSecretHash> {
        s.parse()
    }
}

impl From<SecretHash> for String {
    fn from(h: SecretHash) -> String {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let h = SecretHash::keyed(b"k", "token");
        assert_eq!(h.to_string().parse::<SecretHash>().unwrap(), h);
        assert!("XYZ".parse::<SecretHash>().is_err());
        assert_ne!(h, SecretHash::keyed(b"k", "Token"));
        assert_ne!(h, SecretHash::keyed(b"other", "token"));
    }

    #[test]
    fn join_codes_are_typable_and_normalize() {
        let code = join_code([0xff; 32]);
        assert_eq!(code, "ZZZZ-ZZZZ");
        let code = join_code([7; 32]);
        assert_eq!(code.len(), 9);
        assert!(
            normalize_join_code(&code)
                .bytes()
                .all(|b| CROCKFORD.contains(&b))
        );
        assert_eq!(normalize_join_code(" ab1o-il2z "), "AB10112Z");
        assert_eq!(seal_token([1; 32]).len(), 64);
    }
}
