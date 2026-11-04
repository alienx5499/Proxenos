//! AES-256-GCM limit vault; key from PROXENOS_VAULT_KEY (64 hex chars). Crypto only:
//! the sealed bytes are stored with the deal by the repository.

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use proxenos_app::ports::{LimitVaultPort, VaultError};
use proxenos_app::stored::SealedLimits;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::party::Role;

use crate::crypto::{KeyError, key_from_hex, random};

pub const VAULT_KEY_VAR: &str = "PROXENOS_VAULT_KEY";

/// The 32-byte vault key. Debug never prints it.
pub struct VaultKey([u8; 32]);

impl VaultKey {
    pub fn parse(value: Option<&str>) -> Result<Self, KeyError> {
        key_from_hex(VAULT_KEY_VAR, value).map(Self)
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(redacted)")
    }
}

pub struct AesVault(Aes256Gcm);

impl AesVault {
    pub fn new(key: &VaultKey) -> Self {
        Self(Aes256Gcm::new_from_slice(&key.0).expect("32-byte key"))
    }
}

impl LimitVaultPort for AesVault {
    /// Fresh random 96-bit nonce per call; AAD binds the result to `deal` and `role`.
    fn encrypt(
        &self,
        deal: &DealId,
        role: Role,
        limits: &LimitSet,
    ) -> Result<SealedLimits, VaultError> {
        let plain = serde_json::to_vec(limits).map_err(backend)?;
        let nonce: [u8; 12] = random().map_err(backend)?;
        let aad = aad(deal, role);
        let payload = Payload {
            msg: &plain,
            aad: &aad,
        };
        let ciphertext = self
            .0
            .encrypt(&Nonce::from(nonce), payload)
            .map_err(|_| backend("encryption failed"))?;
        Ok(SealedLimits {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn decrypt(
        &self,
        deal: &DealId,
        role: Role,
        sealed: &SealedLimits,
    ) -> Result<LimitSet, VaultError> {
        let nonce: [u8; 12] = sealed
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::Tampered)?;
        let aad = aad(deal, role);
        let payload = Payload {
            msg: &sealed.ciphertext,
            aad: &aad,
        };
        let plain = self
            .0
            .decrypt(&Nonce::from(nonce), payload)
            .map_err(|_| VaultError::Tampered)?;
        serde_json::from_slice(&plain).map_err(backend)
    }
}

impl fmt::Debug for AesVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AesVault")
    }
}

/// `deal_id ‖ 0x00 ‖ role`, so a ciphertext cannot move between deals or sides.
fn aad(deal: &DealId, role: Role) -> Vec<u8> {
    let mut out = deal.as_str().as_bytes().to_vec();
    out.push(0);
    out.push(match role {
        Role::A => b'A',
        Role::B => b'B',
    });
    out
}

fn backend(e: impl fmt::Display) -> VaultError {
    VaultError::Backend(e.to_string())
}

#[cfg(test)]
#[path = "aes_tests.rs"]
mod tests;
