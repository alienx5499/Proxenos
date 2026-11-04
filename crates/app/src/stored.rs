//! StoredDeal: the aggregate plus app-owned state (secrets, sealed limits, checks,
//! sign requests), saved as one unit.

use proxenos_domain::deal::Deal;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::kind::Field;
use proxenos_domain::party::{ParticipantId, Role};
use proxenos_domain::timestamp::Timestamp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::AppError;
use crate::ports::{DealReader, RequestId};
use crate::receipt::Receipt;
use crate::secret::{SecretHash, hex, unhex};

pub const PROBES_PER_ROUND: u8 = 3;
/// Wrong join codes typed in a side's desk room before join is disabled for that side.
pub const JOIN_ATTEMPTS: u8 = 10;
/// Own-side checks kept per side for the desk `status`.
pub const CHECKS_KEPT: usize = 5;

/// Opaque vault output; serialized as hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedLimits {
    #[serde(serialize_with = "to_hex", deserialize_with = "from_hex")]
    pub nonce: Vec<u8>,
    #[serde(serialize_with = "to_hex", deserialize_with = "from_hex")]
    pub ciphertext: Vec<u8>,
}

fn to_hex<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex(bytes))
}

fn from_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(d)?;
    unhex(&s).ok_or_else(|| serde::de::Error::custom("expected lowercase hex"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Probe,
    Offer,
    Accept,
}

/// An own-side limit check; shown only in that side's desk room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub kind: CheckKind,
    pub round: u16,
    pub within: bool,
    pub failing_fields: Vec<Field>,
    pub at: Timestamp,
}

/// A `None` secret hash means that secret was used (or never issued).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyState {
    pub seal_token: Option<SecretHash>,
    pub join_code: Option<SecretHash>,
    pub failed_joins: u8,
    pub sealed: Option<SealedLimits>,
    pub probes_used: u8,
    pub probes_round: u16,
    /// Newest last, at most `CHECKS_KEPT`.
    pub checks: Vec<Check>,
    /// Ids given at open, bound in the same save as this side's seal.
    #[serde(default)]
    pub pending_binding: Option<PendingBinding>,
    /// The negotiator's chat name, the only name its turn wakes mention.
    #[serde(default)]
    pub negotiator_name: Option<String>,
}

/// Owner and negotiator for a side, named up front by the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingBinding {
    pub owner: ParticipantId,
    pub agent: ParticipantId,
}

impl PartyState {
    pub fn probes_left(&self, round: u16) -> u8 {
        if self.probes_round == round {
            PROBES_PER_ROUND.saturating_sub(self.probes_used)
        } else {
            PROBES_PER_ROUND
        }
    }

    pub fn record_check(&mut self, check: Check) {
        self.checks.push(check);
        let excess = self.checks.len().saturating_sub(CHECKS_KEPT);
        self.checks.drain(..excess);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignRequestState {
    Open,
    Signed,
    Declined,
    /// Answered by someone other than the side's bound owner; the card is closed.
    WrongParty,
    Expired,
    /// A newer card was opened for the same side.
    Superseded,
}

/// A sign card the notary opened for one side and one terms hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignRequest {
    pub request_id: RequestId,
    pub role: Role,
    pub terms_hash: TermsHash,
    pub state: SignRequestState,
    pub answered_by: Option<ParticipantId>,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredDeal {
    pub deal: Deal,
    pub parties: [PartyState; 2],
    pub sign_requests: Vec<SignRequest>,
    /// The Switch group the parties were admitted for.
    pub group_id: Option<String>,
    pub receipt: Option<Receipt>,
    /// The deal-room document holding the deal sheet, once written.
    #[serde(default)]
    pub deal_sheet_doc: Option<String>,
    /// The last deal sheet write failed; retried until it succeeds.
    #[serde(default)]
    pub sheet_dirty: bool,
    /// Number of saves so far; the repository's optimistic concurrency check.
    pub version: u64,
}

impl StoredDeal {
    pub fn new(deal: Deal) -> Self {
        Self {
            deal,
            parties: Default::default(),
            sign_requests: Vec::new(),
            group_id: None,
            receipt: None,
            deal_sheet_doc: None,
            sheet_dirty: false,
            version: 0,
        }
    }

    pub fn party(&self, role: Role) -> &PartyState {
        &self.parties[slot(role)]
    }

    pub fn party_mut(&mut self, role: Role) -> &mut PartyState {
        &mut self.parties[slot(role)]
    }
}

pub fn slot(role: Role) -> usize {
    match role {
        Role::A => 0,
        Role::B => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedEvent {
    pub seq: u64,
    pub at: Timestamp,
    pub event: DealEvent,
}

pub(crate) async fn load_deal(repo: &impl DealReader, id: &DealId) -> Result<StoredDeal, AppError> {
    repo.load(id)
        .await?
        .ok_or_else(|| AppError::DealNotFound(id.clone()))
}
