//! HTTP interface: the seal page and API, the bench's operator endpoints, public read-outs.

mod deals;
mod error;
pub mod handlers;
pub mod routes;
mod seal_page;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_operator;

use proxenos_app::ports::{
    Clock, DealRepository, LimitVaultPort, ReceiptSigner, SecretSource, SwitchAdminPort,
};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::ParticipantId;
use tokio::sync::mpsc;

pub use error::ApiError;

/// Every port the HTTP layer drives.
pub trait HttpPorts: Send + Sync + 'static {
    type Repo: DealRepository;
    type Vault: LimitVaultPort;
    type Clock: Clock;
    type Secrets: SecretSource;
    type Admin: SwitchAdminPort;
    type Signer: ReceiptSigner;
    fn repo(&self) -> &Self::Repo;
    fn vault(&self) -> &Self::Vault;
    fn clock(&self) -> &Self::Clock;
    fn secrets(&self) -> &Self::Secrets;
    fn admin(&self) -> &Self::Admin;
    fn signer(&self) -> &Self::Signer;
}

pub struct HttpConfig {
    /// Bearer token for `POST /api/deals` and `/admit`.
    pub operator_token: String,
    /// Base of the seal links, without a trailing slash.
    pub public_url: String,
    /// Ids that may never negotiate: the notary and registered relays.
    pub excluded: Vec<ParticipantId>,
    /// Deal lifetime for deals opened over HTTP, in seconds.
    pub deal_secs: i64,
    /// Round cap when the request body omits `max_rounds`.
    pub max_rounds: u16,
}

pub struct AppState<P> {
    pub ports: P,
    pub cfg: HttpConfig,
    /// Receives each deal whose second side just sealed, so its first turn can be woken.
    pub opened: Option<mpsc::Sender<DealId>>,
}

impl<P: HttpPorts> AppState<P> {
    pub fn new(ports: P, cfg: HttpConfig) -> Self {
        Self {
            ports,
            cfg,
            opened: None,
        }
    }

    pub fn notify_opened(mut self, tx: mpsc::Sender<DealId>) -> Self {
        self.opened = Some(tx);
        self
    }
}
