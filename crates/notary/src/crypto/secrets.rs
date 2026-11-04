//! Production SecretSource: OS randomness and HMAC under PROXENOS_SECRET_KEY (64 hex chars).

use std::fmt;

use proxenos_app::ports::SecretSource;
use proxenos_app::secret::SecretHash;

use super::{KeyError, key_from_hex, random};

pub const SECRET_KEY_VAR: &str = "PROXENOS_SECRET_KEY";

pub struct OsSecrets([u8; 32]);

impl OsSecrets {
    pub fn parse(value: Option<&str>) -> Result<Self, KeyError> {
        key_from_hex(SECRET_KEY_VAR, value).map(Self)
    }
}

impl fmt::Debug for OsSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OsSecrets(redacted)")
    }
}

impl SecretSource for OsSecrets {
    fn random(&self) -> [u8; 32] {
        // An OS without a working CSPRNG cannot issue secrets safely; stop rather than guess.
        random().expect("system randomness")
    }

    fn hash(&self, secret: &str) -> SecretHash {
        SecretHash::keyed(&self.0, secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_under_the_server_key_and_redacts_it() {
        let key = "0f".repeat(32);
        let s = OsSecrets::parse(Some(&key)).unwrap();
        assert_eq!(s.hash("JC"), SecretHash::keyed(&[0x0f; 32], "JC"));
        assert_ne!(s.hash("JC"), OsSecrets([1; 32]).hash("JC"));
        assert_ne!(s.random(), s.random());
        assert_eq!(format!("{s:?}"), "OsSecrets(redacted)");
        assert!(matches!(
            OsSecrets::parse(None),
            Err(KeyError::Missing(SECRET_KEY_VAR))
        ));
        assert!(matches!(
            OsSecrets::parse(Some("12")),
            Err(KeyError::Malformed(SECRET_KEY_VAR))
        ));
    }
}
