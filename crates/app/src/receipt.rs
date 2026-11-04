//! Receipt: canonical JSON body signed by the notary (ed25519 in production).

use proxenos_domain::deal::{Deal, DealState};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{ParticipantId, Role};
use proxenos_domain::signature::MessageId;
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ports::ReceiptSigner;
use crate::secret::hex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptParty {
    pub role: Role,
    pub owner_id: ParticipantId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptSignature {
    pub role: Role,
    pub signer: ParticipantId,
    pub message_id: MessageId,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptBody {
    pub deal_id: DealId,
    pub kind: DealKind,
    pub terms: Terms,
    pub terms_hash: TermsHash,
    pub parties: Vec<ReceiptParty>,
    pub signatures: Vec<ReceiptSignature>,
    pub bound_at: Timestamp,
    /// Hex of the notary's public key.
    pub notary_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub body: ReceiptBody,
    /// Hex of the signature over `body.canonical_bytes()`.
    pub signature: String,
}

impl ReceiptBody {
    /// Compact JSON with object keys sorted at every level; what gets signed and verified.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let value = serde_json::to_value(self).expect("receipt body is plain JSON");
        serde_json::to_vec(&sorted(value)).expect("JSON value serializes")
    }
}

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let map: Map<String, Value> =
                entries.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            Value::Object(map)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// The receipt for a Bound deal; `None` before that.
pub(crate) fn issue(
    deal: &Deal,
    bound_at: Timestamp,
    signer: &impl ReceiptSigner,
) -> Option<Receipt> {
    let DealState::Bound(terms_hash) = deal.state() else {
        return None;
    };
    let parties = [Role::A, Role::B]
        .into_iter()
        .map(|role| {
            let owner_id = deal.binding(role).map(|b| b.owner.clone())?;
            Some(ReceiptParty { role, owner_id })
        })
        .collect::<Option<Vec<_>>>()?;
    let signatures = deal
        .signatures()
        .iter()
        .map(|s| ReceiptSignature {
            role: s.role,
            signer: s.signer.clone(),
            message_id: s.message_id.clone(),
            at: s.at,
        })
        .collect();
    let body = ReceiptBody {
        deal_id: deal.id().clone(),
        kind: deal.kind(),
        terms: deal.agreed_offer()?.terms.clone(),
        terms_hash,
        parties,
        signatures,
        bound_at,
        notary_key: hex(&signer.public_key()),
    };
    let signature = hex(&signer.sign(&body.canonical_bytes()));
    Some(Receipt { body, signature })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_nested_keys() {
        let v = serde_json::json!({"b": 1, "a": {"z": [{"y": 1, "x": 2}], "c": 3}});
        let s = serde_json::to_string(&sorted(v)).unwrap();
        assert_eq!(s, r#"{"a":{"c":3,"z":[{"x":2,"y":1}]},"b":1}"#);
    }
}
