//! Signature { role, terms_hash, message_id, signer, at }.

use serde::{Deserialize, Serialize};

use crate::hash::TermsHash;
use crate::id::non_empty_id;
use crate::party::{ParticipantId, Role};
use crate::timestamp::Timestamp;

non_empty_id!(
    /// The chat message id or sign-card request id that carried the signature.
    MessageId
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub role: Role,
    pub terms_hash: TermsHash,
    pub message_id: MessageId,
    pub signer: ParticipantId,
    pub at: Timestamp,
}
