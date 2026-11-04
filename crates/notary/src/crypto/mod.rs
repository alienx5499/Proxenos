//! Receipt cryptography and the key parsing shared with the vault.

pub mod receipt;
pub mod secrets;

use thiserror::Error;

/// Startup failure loading a secret key; the message never contains key material.
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{0} must be 64 hex characters (32 bytes)")]
    Malformed(&'static str),
    #[error("key file {0} is readable by group or others; run `chmod 600 {0}`")]
    Exposed(String),
    #[error("key file {path}: {source}")]
    File {
        path: String,
        source: std::io::Error,
    },
    #[error("system randomness: {0}")]
    Random(String),
}

pub use proxenos_app::secret::unhex_array as from_hex;

/// A 32-byte key from a hex variable; `None` means the variable is unset.
pub fn key_from_hex(var: &'static str, value: Option<&str>) -> Result<[u8; 32], KeyError> {
    let value = value.ok_or(KeyError::Missing(var))?;
    from_hex(value).ok_or(KeyError::Malformed(var))
}

pub fn random<const N: usize>() -> Result<[u8; N], KeyError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| KeyError::Random(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_exact_length_only() {
        assert_eq!(from_hex::<2>("0aFf"), Some([0x0a, 0xff]));
        assert_eq!(from_hex::<2>("0a"), None);
        assert_eq!(from_hex::<2>("0g00"), None);
        assert_eq!(from_hex::<2>("+a00"), None);
    }
}
