//! The production ports bundle: one value serves both the HTTP layer and the chat router.

use std::sync::Arc;

use proxenos_app::ports::RepoError;
use proxenos_domain::timestamp::Timestamp;

use crate::adapters::SystemClock;
use crate::adapters::switch::SwitchAdapter;
use crate::chat::router::{Ports, SeenMessages};
use crate::crypto::receipt::Ed25519Signer;
use crate::crypto::secrets::OsSecrets;
use crate::http::HttpPorts;
use crate::store::sqlite::Store;
use crate::switch_admin::GatewayAdmin;
use crate::vault::aes::AesVault;

pub struct Parts {
    pub store: Store,
    pub vault: AesVault,
    pub clock: SystemClock,
    pub secrets: OsSecrets,
    pub signer: Ed25519Signer,
    pub switch: SwitchAdapter,
    pub admin: GatewayAdmin,
}

/// Cheap to clone; the router and the HTTP state each hold one.
#[derive(Clone)]
pub struct NotaryPorts(pub Arc<Parts>);

impl SeenMessages for Store {
    async fn mark_seen(&self, message_id: &str, at: Timestamp) -> Result<bool, RepoError> {
        Store::mark_seen(self, message_id, at).await
    }
}

impl Ports for NotaryPorts {
    type Repo = Store;
    type Vault = AesVault;
    type Clock = SystemClock;
    type Secrets = OsSecrets;
    type Signer = Ed25519Signer;
    type Switch = SwitchAdapter;
    type Roster = SwitchAdapter;
    type Seen = Store;
    fn repo(&self) -> &Store {
        &self.0.store
    }
    fn vault(&self) -> &AesVault {
        &self.0.vault
    }
    fn clock(&self) -> &SystemClock {
        &self.0.clock
    }
    fn secrets(&self) -> &OsSecrets {
        &self.0.secrets
    }
    fn signer(&self) -> &Ed25519Signer {
        &self.0.signer
    }
    fn switch(&self) -> &SwitchAdapter {
        &self.0.switch
    }
    fn roster(&self) -> &SwitchAdapter {
        &self.0.switch
    }
    fn seen(&self) -> &Store {
        &self.0.store
    }
}

impl HttpPorts for NotaryPorts {
    type Repo = Store;
    type Vault = AesVault;
    type Clock = SystemClock;
    type Secrets = OsSecrets;
    type Admin = GatewayAdmin;
    type Signer = Ed25519Signer;
    fn repo(&self) -> &Store {
        &self.0.store
    }
    fn vault(&self) -> &AesVault {
        &self.0.vault
    }
    fn clock(&self) -> &SystemClock {
        &self.0.clock
    }
    fn secrets(&self) -> &OsSecrets {
        &self.0.secrets
    }
    fn admin(&self) -> &GatewayAdmin {
        &self.0.admin
    }
    fn signer(&self) -> &Ed25519Signer {
        &self.0.signer
    }
}
