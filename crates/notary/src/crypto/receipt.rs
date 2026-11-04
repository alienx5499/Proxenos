//! ed25519 receipt signing; key from PROXENOS_RECEIPT_KEY or a persisted key file.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use proxenos_app::ports::ReceiptSigner;
use proxenos_app::secret::hex;
use thiserror::Error;

use super::{KeyError, from_hex, key_from_hex, random};

pub const RECEIPT_KEY_VAR: &str = "PROXENOS_RECEIPT_KEY";

pub struct Ed25519Signer(SigningKey);

impl Ed25519Signer {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }

    /// Seed from `env_seed` (64 hex chars) if set; else read from `key_file` (same format,
    /// refused unless mode 0600); else generate one and create `key_file` atomically with
    /// mode 0600, so the public key is stable across restarts.
    pub fn load(env_seed: Option<&str>, key_file: &Path) -> Result<Self, KeyError> {
        if env_seed.is_some() {
            return key_from_hex(RECEIPT_KEY_VAR, env_seed).map(Self::from_seed);
        }
        if let Some(seed) = read_key_file(key_file)? {
            return Ok(Self::from_seed(seed));
        }
        let seed = random::<32>()?;
        let temp = key_file.with_extension(format!("tmp-{}", hex(&random::<8>()?)));
        match create_key_file(&temp, key_file, &seed) {
            Ok(()) => Ok(Self::from_seed(seed)),
            // Another process created it first: use theirs.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => read_key_file(key_file)?
                .map(Self::from_seed)
                .ok_or_else(|| file_err(key_file, e)),
            Err(e) => Err(file_err(key_file, e)),
        }
    }

    pub fn public_key_hex(&self) -> String {
        hex(self.0.verifying_key().as_bytes())
    }
}

fn file_err(path: &Path, source: io::Error) -> KeyError {
    KeyError::File {
        path: path.display().to_string(),
        source,
    }
}

/// `None` when the file does not exist.
fn read_key_file(path: &Path) -> Result<Option<[u8; 32]>, KeyError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(file_err(path, e)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = file.metadata().map_err(|e| file_err(path, e))?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(KeyError::Exposed(path.display().to_string()));
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| file_err(path, e))?;
    from_hex(&text)
        .map(Some)
        .ok_or(KeyError::Malformed("receipt key file"))
}

/// Writes a 0600 temp file in the same directory, then hard-links it into place, which
/// fails with `AlreadyExists` instead of overwriting a key another process just wrote.
fn create_key_file(temp: &Path, path: &Path, seed: &[u8; 32]) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let written = opts.open(temp).and_then(|mut file| {
        file.write_all(hex(seed).as_bytes())?;
        file.sync_all()
    });
    let linked = written.and_then(|()| fs::hard_link(temp, path));
    let _ = fs::remove_file(temp);
    linked
}

impl fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ed25519Signer(public {})", self.public_key_hex())
    }
}

impl ReceiptSigner for Ed25519Signer {
    fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.0.sign(bytes).to_bytes().to_vec()
    }

    fn public_key(&self) -> Vec<u8> {
        self.0.verifying_key().to_bytes().to_vec()
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum VerifyError {
    #[error("public key must be 64 hex characters of a valid ed25519 key")]
    BadKey,
    #[error("signature must be 128 hex characters")]
    BadSignature,
    #[error("signature does not match the receipt body")]
    Mismatch,
}

/// Offline check of a receipt: `body` is `ReceiptBody::canonical_bytes()`.
pub fn verify(body: &[u8], sig_hex: &str, pubkey_hex: &str) -> Result<(), VerifyError> {
    let key = from_hex::<32>(pubkey_hex).ok_or(VerifyError::BadKey)?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| VerifyError::BadKey)?;
    let sig = from_hex::<64>(sig_hex).ok_or(VerifyError::BadSignature)?;
    key.verify_strict(body, &Signature::from_bytes(&sig))
        .map_err(|_| VerifyError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> Ed25519Signer {
        Ed25519Signer::from_seed([7; 32])
    }

    #[test]
    fn signature_verifies_and_tampering_fails() {
        let s = signer();
        let body = br#"{"deal_id":"D-0001"}"#;
        let sig = hex(&s.sign(body));
        let key = s.public_key_hex();
        assert_eq!(verify(body, &sig, &key), Ok(()));
        assert_eq!(
            verify(br#"{"deal_id":"D-0002"}"#, &sig, &key),
            Err(VerifyError::Mismatch)
        );
        let other = Ed25519Signer::from_seed([8; 32]).public_key_hex();
        assert_eq!(verify(body, &sig, &other), Err(VerifyError::Mismatch));
        let mut flipped = sig.clone().into_bytes();
        flipped[0] = if flipped[0] == b'0' { b'1' } else { b'0' };
        let flipped = String::from_utf8(flipped).unwrap();
        assert_eq!(verify(body, &flipped, &key), Err(VerifyError::Mismatch));
        assert_eq!(verify(body, "zz", &key), Err(VerifyError::BadSignature));
        assert_eq!(verify(body, &sig, "00"), Err(VerifyError::BadKey));
    }

    #[test]
    fn key_comes_from_env_then_file_and_persists() {
        let dir = std::env::temp_dir().join(format!("proxenos-key-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("receipt.key");
        let _ = fs::remove_file(&path);

        let first = Ed25519Signer::load(None, &path).unwrap();
        let again = Ed25519Signer::load(None, &path).unwrap();
        assert_eq!(first.public_key(), again.public_key());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "temp file removed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            let err = Ed25519Signer::load(None, &path).unwrap_err();
            assert!(matches!(err, KeyError::Exposed(_)));
            assert!(err.to_string().contains("chmod 600"));
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let seed = hex(&[7; 32]);
        let from_env = Ed25519Signer::load(Some(&seed), &path).unwrap();
        assert_eq!(from_env.public_key(), signer().public_key());
        assert!(matches!(
            Ed25519Signer::load(Some("abc"), &path),
            Err(KeyError::Malformed(RECEIPT_KEY_VAR))
        ));
        assert!(!format!("{first:?}").contains(&hex(&first.0.to_bytes())));
        fs::remove_dir_all(&dir).unwrap();
    }
}
