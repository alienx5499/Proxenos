//! Refusals on limits and seal tokens; own-side checks; repository concurrency.

use proxenos_domain::event::DealEvent;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::commands::accept_offer::AcceptOutcome;
use crate::commands::submit_offer::OfferOutcome;
use crate::ports::{DealReader, DealRepository, LimitVaultPort, RepoError, VaultError};
use crate::queries::get_seal_form::get_seal_form;
use crate::queries::get_status::get_status;
use crate::stored::CheckKind;

const D: &str = "D-0007";

#[tokio::test]
async fn offer_and_accept_outside_own_limits_are_refused_and_logged() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let out = h.offer(D, Role::A, po(120_000, 400, 560, 35)).await;
    let price = vec![Field::PricePerKg];
    assert_eq!(
        out,
        Ok(OfferOutcome::RefusedInDesk {
            failing_fields: price.clone()
        })
    );
    let out = h
        .offer_in(D, Role::A, po(120_000, 400, 560, 35), deal_room())
        .await;
    let Ok(OfferOutcome::RefusedInDealRoom { desk_detail }) = out else {
        panic!("expected a deal-room refusal, got {out:?}");
    };
    assert_eq!(desk_detail.for_desk(), price);
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert!(stored.deal.offers().is_empty());
    let last = h.repo.events(&id(D)).await.unwrap().pop().unwrap();
    assert_eq!(last.event, DealEvent::OfferRefused { role: Role::A });

    h.offer(D, Role::A, po(98_000, 400, 560, 35)).await.unwrap();
    let out = h.accept_in(D, Role::B, "O-1", deal_room()).await;
    let Ok(AcceptOutcome::RefusedInDealRoom { desk_detail }) = out else {
        panic!("expected a deal-room refusal, got {out:?}");
    };
    assert_eq!(desk_detail.for_desk(), price);
    let out = h.accept(D, Role::B, "O-1").await;
    assert_eq!(
        out,
        Ok(AcceptOutcome::RefusedInDesk {
            failing_fields: price.clone()
        })
    );
    let last = h.repo.events(&id(D)).await.unwrap().pop().unwrap();
    assert_eq!(last.event, DealEvent::AcceptRefused { role: Role::B });

    let desk_a = get_status(&h.repo, &id(D), Some(Role::A)).await.unwrap();
    let checks = desk_a.last_checks.unwrap();
    let kinds: Vec<_> = checks.iter().map(|c| (c.kind, c.within)).collect();
    let offer = CheckKind::Offer;
    assert_eq!(kinds, vec![(offer, false), (offer, false), (offer, true)]);
    assert_eq!(checks[0].failing_fields, price);
    let public = get_status(&h.repo, &id(D), None).await.unwrap();
    assert_eq!((public.last_checks, public.probes_left), (None, None));
}

#[tokio::test]
async fn checks_are_bounded() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    for _ in 0..7 {
        h.offer(D, Role::A, po(120_000, 400, 560, 35))
            .await
            .unwrap();
    }
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert_eq!(
        stored.party(Role::A).checks.len(),
        crate::stored::CHECKS_KEPT
    );
}

#[tokio::test]
async fn seal_token_is_single_use_and_side_bound() {
    let h = H::default();
    let tokens = h.open(D, DealKind::PurchaseOrder).await;
    assert_eq!(tokens.role_a.len(), 64);
    let form = get_seal_form(&h.repo, &h.secrets, &tokens.role_b)
        .await
        .unwrap();
    assert_eq!((form.role, form.kind), (Role::B, DealKind::PurchaseOrder));
    let po_limits = |r| limits(DealKind::PurchaseOrder, r);
    let wrong = h.seal(&tokens.role_a, po_limits(Role::B)).await;
    assert_eq!(wrong.err(), Some(AppError::WrongSide));
    h.seal(&tokens.role_a, po_limits(Role::A)).await.unwrap();
    let again = h.seal(&tokens.role_a, po_limits(Role::A)).await;
    assert_eq!(again.err(), Some(AppError::UnknownSealToken));
    let form = get_seal_form(&h.repo, &h.secrets, &tokens.role_a).await;
    assert_eq!(form, Err(AppError::UnknownSealToken));
}

#[tokio::test]
async fn sealed_limits_live_on_the_deal_and_bind_to_deal_and_role() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    let sealed = stored.party(Role::A).sealed.clone().unwrap();
    let own = h.vault.decrypt(&id(D), Role::A, &sealed).unwrap();
    assert_eq!(own, limits(DealKind::PurchaseOrder, Role::A));
    assert_eq!(
        h.vault.decrypt(&id(D), Role::B, &sealed),
        Err(VaultError::Tampered)
    );
    assert_eq!(
        h.vault.decrypt(&id("D-0001"), Role::A, &sealed),
        Err(VaultError::Tampered)
    );
}

#[tokio::test]
async fn stale_save_conflicts() {
    let h = H::default();
    h.open(D, DealKind::PurchaseOrder).await;
    let mut stale = h.repo.load(&id(D)).await.unwrap().unwrap();
    stale.version -= 1;
    assert_eq!(
        h.repo.save(&stale, &[], Timestamp(1)).await,
        Err(RepoError::Conflict)
    );
}
