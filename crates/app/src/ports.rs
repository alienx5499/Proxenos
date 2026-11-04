//! Ports: the app's only way out. Production adapters live in the notary, fakes in `fakes`.

use std::future::Future;

use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::secret::SecretHash;
use crate::stored::{LoggedEvent, SealedLimits, StoredDeal};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepoError {
    #[error("deal changed since it was loaded")]
    Conflict,
    #[error("store: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VaultError {
    #[error("sealed limits failed authentication")]
    Tampered,
    #[error("vault: {0}")]
    Backend(String),
}

/// Any failure talking to Switch (agent or admin API).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("switch: {0}")]
pub struct SwitchError(pub String);

/// Read side of the deal store; queries depend on this alone.
pub trait DealReader: Send + Sync {
    fn load(
        &self,
        id: &DealId,
    ) -> impl Future<Output = Result<Option<StoredDeal>, RepoError>> + Send;
    /// Deals whose deal room or either desk room is `room`.
    fn in_room(
        &self,
        room: &RoomId,
    ) -> impl Future<Output = Result<Vec<StoredDeal>, RepoError>> + Send;
    /// Ids of deals not yet Bound, Withdrawn or Expired.
    fn active(&self) -> impl Future<Output = Result<Vec<DealId>, RepoError>> + Send;
    /// Ids of deals, closed ones included, whose last deal sheet write failed.
    fn dirty_sheets(&self) -> impl Future<Output = Result<Vec<DealId>, RepoError>> + Send;
    fn by_seal_token(
        &self,
        hash: &SecretHash,
    ) -> impl Future<Output = Result<Option<(DealId, Role)>, RepoError>> + Send;
    fn by_join_code(
        &self,
        hash: &SecretHash,
    ) -> impl Future<Output = Result<Option<(DealId, Role)>, RepoError>> + Send;
    /// Id of the active deal holding sign request `request`.
    fn by_sign_request(
        &self,
        request: &RequestId,
    ) -> impl Future<Output = Result<Option<DealId>, RepoError>> + Send;
    fn events(
        &self,
        id: &DealId,
    ) -> impl Future<Output = Result<Vec<LoggedEvent>, RepoError>> + Send;
}

pub trait DealRepository: DealReader {
    /// Writes `deal` and appends `events` stamped `at`, in one transaction.
    /// `Conflict` unless the stored version still equals `deal.version` (0 for a new deal).
    fn save(
        &self,
        deal: &StoredDeal,
        events: &[DealEvent],
        at: Timestamp,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
}

/// Limit encryption only (AES-256-GCM in production, AAD = deal id and role).
/// Ciphertext lives on `StoredDeal`, saved with the deal; use cases alone call `decrypt`.
pub trait LimitVaultPort: Send + Sync {
    fn encrypt(
        &self,
        deal: &DealId,
        role: Role,
        limits: &LimitSet,
    ) -> Result<SealedLimits, VaultError>;
    /// `Tampered` when the ciphertext fails authentication, including for another deal or role.
    fn decrypt(
        &self,
        deal: &DealId,
        role: Role,
        sealed: &SealedLimits,
    ) -> Result<LimitSet, VaultError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

/// What happened to a sign card; the adapter maps Switch's option to these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardAnswer {
    Sign(ParticipantId),
    Decline(ParticipantId),
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardOutcome {
    pub request: RequestId,
    pub answer: CardAnswer,
}

/// One side's sign card, in the deal room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignCard {
    pub room: RoomId,
    pub deal: DealId,
    pub role: Role,
    /// The side's label for the deal kind, e.g. `buyer`.
    pub label: String,
    pub terms_hash: TermsHash,
    pub round: u16,
    pub expires: Timestamp,
    /// 1 for the side's first card on these terms; card ids must stay unique.
    pub attempt: usize,
}

/// Commands to Switch as the notary agent. The sign-card pair is conditional core.
pub trait SwitchPort: Send + Sync {
    fn post(
        &self,
        room: &RoomId,
        text: &str,
    ) -> impl Future<Output = Result<(), SwitchError>> + Send;
    fn write_document(
        &self,
        room: &RoomId,
        name: &str,
        content: &str,
    ) -> impl Future<Output = Result<DocId, SwitchError>> + Send;
    fn update_document(
        &self,
        room: &RoomId,
        doc: &DocId,
        content: &str,
    ) -> impl Future<Output = Result<(), SwitchError>> + Send;
    fn request_signature(
        &self,
        card: &SignCard,
    ) -> impl Future<Output = Result<RequestId, SwitchError>> + Send;
    fn ack_outcome(
        &self,
        request: &RequestId,
    ) -> impl Future<Output = Result<(), SwitchError>> + Send;
}

/// Polled sign-card results (conditional core); acking is on `SwitchPort`.
pub trait SignatureOutcomes: Send + Sync {
    fn undelivered(&self) -> impl Future<Output = Result<Vec<CardOutcome>, SwitchError>> + Send;
}

/// Switch admin calls as the operator.
pub trait SwitchAdminPort: Send + Sync {
    /// Adds one rule to the notary's policy; never replaces existing rules.
    fn admit_parties(
        &self,
        group: &str,
        users: &[String],
        agents: &[String],
    ) -> impl Future<Output = Result<(), SwitchError>> + Send;
    fn group_id(&self, name: &str) -> impl Future<Output = Result<String, SwitchError>> + Send;
    fn external_user_id(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<String, SwitchError>> + Send;
}

/// Names only, for addressing a wake; never used for identity.
pub trait RosterPort: Send + Sync {
    fn agent_names(
        &self,
        room: &RoomId,
    ) -> impl Future<Output = Result<Vec<String>, SwitchError>> + Send;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

pub trait ReceiptSigner: Send + Sync {
    fn sign(&self, bytes: &[u8]) -> Vec<u8>;
    fn public_key(&self) -> Vec<u8>;
}

/// Randomness and the server-keyed hash for seal tokens and join codes.
/// Token and code formats are built in `secret`; the adapter only supplies these two.
pub trait SecretSource: Send + Sync {
    /// 32 bytes from a cryptographic RNG.
    fn random(&self) -> [u8; 32];
    /// `SecretHash::keyed(server_key, secret)` with the adapter's server key.
    fn hash(&self, secret: &str) -> SecretHash;
}
