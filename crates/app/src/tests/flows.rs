//! Full PO and lease flows to Bound with a verifiable receipt.

use proxenos_domain::deal::DealState;
use proxenos_domain::event::DealEvent;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::party::Role;

use super::fixtures::*;
use super::support::*;
use crate::commands::accept_offer::AcceptOutcome;
use crate::commands::record_signature::SignOutcome;
use crate::commands::submit_offer::OfferOutcome;
use crate::fakes::FakeSigner;
use crate::ports::{CardAnswer, DealReader};
use crate::queries::StateName;
use crate::queries::get_receipt::get_receipt;
use crate::queries::get_status::get_status;
use crate::receipt::Receipt;
use crate::secret::hex;

fn check_receipt(r: &Receipt, deal: &str, kind: DealKind) {
    assert_eq!(r.body.deal_id, id(deal));
    assert_eq!(r.body.kind, kind);
    assert_eq!(r.body.parties.len(), 2);
    assert_eq!(r.body.parties[0].owner_id, owner(Role::A));
    assert_eq!(r.body.signatures.len(), 2);
    assert_eq!(r.body.notary_key, hex(FakeSigner::PUBLIC_KEY));
    assert_eq!(
        r.signature,
        hex(&FakeSigner::expected(&r.body.canonical_bytes()))
    );
}

#[tokio::test]
async fn purchase_order_binds_with_receipt() {
    let h = H::default();
    h.ready("D-0007", DealKind::PurchaseOrder).await;
    let d = "D-0007";
    assert!(matches!(
        h.offer(d, Role::A, po(98_000, 400, 560, 35)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    let refused = h
        .offer(d, Role::B, po(112_000, 400, 560, 60))
        .await
        .unwrap();
    assert_eq!(
        refused,
        OfferOutcome::RefusedInDesk {
            failing_fields: vec![Field::PaymentNetDays]
        }
    );
    assert!(matches!(
        h.offer(d, Role::B, po(112_000, 400, 560, 40)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    assert!(matches!(
        h.offer(d, Role::A, po(108_000, 400, 560, 40)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    let AcceptOutcome::Agreed { hash } = h.accept(d, Role::B, "O-3").await.unwrap() else {
        panic!("expected agreement");
    };
    let status = get_status(&h.repo, &id(d), None).await.unwrap();
    assert_eq!(status.hash8, Some(hash.short()));

    let first = h.sign(d, owner(Role::A), &hash.short()).await.unwrap();
    assert_eq!(first, SignOutcome::Signed { role: Role::A });
    let card = h.open_card(d, Role::B).await.unwrap();
    let signed = h.card(d, card.clone(), CardAnswer::Sign(owner(Role::B)));
    let SignOutcome::Bound { role, receipt } = signed.await.unwrap() else {
        panic!("expected bound");
    };
    assert_eq!(role, Role::B);
    assert_eq!(receipt.body.signatures[1].message_id.as_str(), card.0);
    check_receipt(&receipt, d, DealKind::PurchaseOrder);
    assert_eq!(receipt.body.terms, po(108_000, 400, 560, 40));
    assert_eq!(receipt.body.terms_hash, hash);
    assert_eq!(
        get_receipt(&h.repo, &id(d)).await.unwrap(),
        Some(*receipt.clone())
    );

    let stored = h.repo.load(&id(d)).await.unwrap().unwrap();
    assert_eq!(stored.deal.state(), DealState::Bound(hash));
    let events = h.repo.events(&id(d)).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event == DealEvent::OfferRefused { role: Role::B })
    );
    assert_eq!(events.last().unwrap().event, DealEvent::Bound { hash });
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());
    assert_eq!(
        get_status(&h.repo, &id(d), None).await.unwrap().state,
        StateName::Bound
    );
}

#[tokio::test]
async fn lease_binds_with_receipt() {
    let h = H::default();
    let d = "D-0009";
    h.ready(d, DealKind::Lease).await;
    assert!(matches!(
        h.offer(d, Role::A, lease(2_600_000, 2, 35, 12)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    assert!(matches!(
        h.offer(d, Role::B, lease(2_900_000, 3, 35, 12)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    assert!(matches!(
        h.offer(d, Role::A, lease(2_800_000, 2, 35, 12)).await,
        Ok(OfferOutcome::Made { .. })
    ));
    let AcceptOutcome::Agreed { hash } = h.accept(d, Role::B, "O-3").await.unwrap() else {
        panic!("expected agreement");
    };
    h.sign(d, owner(Role::B), &hash.to_string()).await.unwrap();
    let SignOutcome::Bound { receipt, .. } =
        h.sign(d, owner(Role::A), &hash.short()).await.unwrap()
    else {
        panic!("expected bound");
    };
    check_receipt(&receipt, d, DealKind::Lease);
    assert_eq!(receipt.body.terms, lease(2_800_000, 2, 35, 12));
}
