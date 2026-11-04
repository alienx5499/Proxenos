//! `proxenos verify <receipt.json>`: offline signature check over the canonical body.

use std::path::PathBuf;

use proxenos_app::receipt::Receipt;

use super::CliError;
use crate::crypto::receipt::verify;

#[derive(Debug, clap::Args)]
pub struct VerifyArgs {
    pub receipt: PathBuf,
    /// Expected notary public key (hex, from `GET /api/notary/key`); defaults to the key in the file.
    #[arg(long)]
    pub key: Option<String>,
}

/// `Ok` names what was proven; with no `--key` only integrity under the embedded key is.
pub fn check(text: &str, key: Option<&str>) -> Result<String, CliError> {
    let receipt: Receipt =
        serde_json::from_str(text).map_err(|e| CliError(format!("not a receipt: {e}")))?;
    let body = &receipt.body;
    let pubkey = key.unwrap_or(&body.notary_key);
    if key.is_some_and(|k| !k.trim().eq_ignore_ascii_case(&body.notary_key)) {
        return Err(CliError("receipt names a different notary key".into()));
    }
    verify(&body.canonical_bytes(), &receipt.signature, pubkey).map_err(CliError::new)?;
    let scope = if key.is_some() {
        "signed by the given notary key"
    } else {
        "consistent with its embedded key (pass --key to check who signed)"
    };
    Ok(format!(
        "valid: {} {} terms {} {scope}",
        body.deal_id,
        body.kind.name(),
        body.terms_hash.short()
    ))
}

pub fn run(args: &VerifyArgs) -> Result<String, CliError> {
    let text = std::fs::read_to_string(&args.receipt)
        .map_err(|e| CliError(format!("{}: {e}", args.receipt.display())))?;
    check(&text, args.key.as_deref())
}

#[cfg(test)]
mod tests {
    use proxenos_app::ports::ReceiptSigner;
    use proxenos_app::receipt::{ReceiptBody, ReceiptParty};
    use proxenos_app::secret::hex;
    use proxenos_domain::date::Date;
    use proxenos_domain::kind::DealKind;
    use proxenos_domain::money::{Currency, Money};
    use proxenos_domain::party::{ParticipantId, Role};
    use proxenos_domain::quantity::Quantity;
    use proxenos_domain::terms::{PurchaseOrderTerms, Terms};
    use proxenos_domain::timestamp::Timestamp;

    use super::*;
    use crate::crypto::receipt::Ed25519Signer;

    fn signed() -> (String, String) {
        let signer = Ed25519Signer::from_seed([9; 32]);
        let terms = PurchaseOrderTerms::new(
            Money::new(109_000, Currency::Inr),
            Quantity::kg(400),
            Date::from_days(35),
            45,
        )
        .unwrap();
        let body = ReceiptBody {
            deal_id: "D-0007".parse().unwrap(),
            kind: DealKind::PurchaseOrder,
            terms_hash: "ab".repeat(32).parse().unwrap(),
            terms: Terms::PurchaseOrder(terms),
            parties: vec![ReceiptParty {
                role: Role::A,
                owner_id: ParticipantId::new("u-asha").unwrap(),
            }],
            signatures: vec![],
            bound_at: Timestamp(1_000),
            notary_key: signer.public_key_hex(),
        };
        let signature = hex(&signer.sign(&body.canonical_bytes()));
        let text = serde_json::to_string_pretty(&Receipt { body, signature }).unwrap();
        (text, signer.public_key_hex())
    }

    #[test]
    fn accepts_a_genuine_receipt_and_rejects_tampering() {
        let (text, key) = signed();
        assert!(
            check(&text, Some(&key))
                .unwrap()
                .contains("given notary key")
        );
        assert!(check(&text, None).unwrap().contains("--key"));
        let tampered = text.replace("109000", "100000");
        assert_ne!(tampered, text);
        assert!(check(&tampered, Some(&key)).is_err());
        let other = Ed25519Signer::from_seed([1; 32]).public_key_hex();
        assert!(check(&text, Some(&other)).is_err());
        assert!(check("{}", None).is_err());
    }
}
