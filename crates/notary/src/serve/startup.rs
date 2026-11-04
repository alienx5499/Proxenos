//! Startup checks against the existing database, before anything listens.

use std::path::Path;

use proxenos_app::ports::LimitVaultPort;

use crate::config::Config;
use crate::crypto::receipt::Ed25519Signer;
use crate::store::sqlite::Store;
use crate::vault::aes::AesVault;

/// The path as the process resolves it, for startup lines and errors.
pub(super) fn shown(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_owned())
        .display()
        .to_string()
}

/// Refuses to mint a new receipt key when Bound deals hold receipts signed by the old one.
pub(super) async fn load_signer(cfg: &Config, store: &Store) -> Result<Ed25519Signer, String> {
    let file = &cfg.receipt_key_file;
    if cfg.receipt_key.is_none() && !file.exists() {
        let bound = store.bound_count().await.map_err(|e| e.to_string())?;
        if bound > 0 {
            return Err(format!(
                "receipt key file {} is missing but {bound} bound deals in {} hold receipts \
                 signed with it; restore the file or set PROXENOS_RECEIPT_KEY",
                shown(file),
                shown(&cfg.db)
            ));
        }
    }
    Ed25519Signer::load(cfg.receipt_key.as_ref().map(|k| k.expose()), file)
        .map_err(|e| e.to_string())
}

/// Opens one stored sealed limit set, so a wrong vault key fails here, not per move.
pub(super) async fn check_vault(
    cfg: &Config,
    store: &Store,
    vault: &AesVault,
) -> Result<(), String> {
    let Some((deal, role, sealed)) = store.sealed_sample().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    vault.decrypt(&deal, role, &sealed).map(drop).map_err(|_| {
        format!(
            "PROXENOS_VAULT_KEY does not open the sealed limits of {deal} in {}; \
             start with the key those deals were sealed under",
            shown(&cfg.db)
        )
    })
}
