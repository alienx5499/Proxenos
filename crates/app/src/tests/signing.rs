//! Signatures: typed `sign` and sign cards, including declines and wrong parties.

use proxenos_domain::error::DomainError;
use proxenos_domain::hash::TermsHash;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::commands::accept_offer::AcceptOutcome;
use crate::commands::record_signature::SignOutcome;
use crate::fakes::SwitchCall;
use crate::ports::{
    CardAnswer, CardOutcome, Clock, DealReader, RequestId, SignatureOutcomes, SwitchPort,
};
use crate::stored::SignRequestState;

const D: &str = "D-0007";

async fn agreed(h: &H) -> TermsHash {
    h.ready(D, DealKind::PurchaseOrder).await;
    h.offer(D, Role::A, po(108_000, 400, 560, 40))
        .await
        .unwrap();
    match h.accept(D, Role::B, "O-1").await.unwrap() {
        AcceptOutcome::Agreed { hash } => hash,
        other => panic!("expected agreement, got {other:?}"),
    }
}

async fn request_state(h: &H, request: &RequestId) -> SignRequestState {
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    let r = stored
        .sign_requests
        .iter()
        .find(|r| &r.request_id == request);
    r.unwrap().state
}

#[tokio::test]
async fn typed_signature_by_non_owner_is_refused() {
    let h = H::default();
    let hash = agreed(&h).await;
    let not_owner = Err(AppError::Domain(DomainError::NotBoundOwner));
    assert_eq!(h.sign(D, agent(Role::A), &hash.short()).await, not_owner);
    assert_eq!(h.sign(D, pid("stranger"), &hash.short()).await, not_owner);
    let mismatch = Err(AppError::Domain(DomainError::HashMismatch));
    assert_eq!(h.sign(D, owner(Role::A), "00000000").await, mismatch);
    assert_eq!(
        h.sign(D, owner(Role::A), &hash.short()[..4]).await,
        mismatch
    );
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert!(stored.deal.signatures().is_empty());
}

#[tokio::test]
async fn card_decline_records_nothing() {
    let h = H::default();
    let hash = agreed(&h).await;
    let card = h.open_card(D, Role::A).await.unwrap();
    let [SwitchCall::RequestSignature(issued)] = &h.switch.calls()[..] else {
        panic!("expected one sign card");
    };
    assert_eq!(issued.room, deal_room());
    assert_eq!((issued.deal.clone(), issued.role), (id(D), Role::A));
    assert_eq!(issued.label, "buyer");
    assert_eq!(issued.terms_hash, hash);
    assert_eq!(issued.attempt, 1);
    assert_eq!(
        issued.expires,
        DEADLINE.min(Timestamp(h.clock.now().0 + 86_400))
    );
    let out = h
        .card(D, card.clone(), CardAnswer::Decline(owner(Role::A)))
        .await;
    assert_eq!(out, Ok(SignOutcome::Declined { role: Role::A }));
    assert_eq!(request_state(&h, &card).await, SignRequestState::Declined);
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert!(stored.deal.signatures().is_empty());
    let replay = h.card(D, card, CardAnswer::Sign(owner(Role::A))).await;
    assert_eq!(replay, Err(AppError::UnknownSignRequest));
}

#[tokio::test]
async fn card_answered_by_wrong_party_closes_it() {
    let h = H::default();
    agreed(&h).await;
    let card = h.open_card(D, Role::A).await.unwrap();
    let out = h
        .card(D, card.clone(), CardAnswer::Sign(owner(Role::B)))
        .await;
    assert_eq!(out, Ok(SignOutcome::WrongParty { role: Role::A }));
    assert_eq!(request_state(&h, &card).await, SignRequestState::WrongParty);
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert!(stored.deal.signatures().is_empty());

    let fresh = h.open_card(D, Role::A).await.unwrap();
    assert_ne!(fresh, card);
    let attempts: Vec<usize> = h
        .switch
        .calls()
        .iter()
        .filter_map(|c| match c {
            SwitchCall::RequestSignature(card) => Some(card.attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, [1, 2]);
    let out = h.card(D, fresh, CardAnswer::Sign(owner(Role::A))).await;
    assert_eq!(out, Ok(SignOutcome::Signed { role: Role::A }));
}

#[tokio::test]
async fn only_issued_open_cards_count() {
    let h = H::default();
    agreed(&h).await;
    let forged = h
        .card(
            D,
            RequestId("req-forged".into()),
            CardAnswer::Sign(owner(Role::A)),
        )
        .await;
    assert_eq!(forged, Err(AppError::UnknownSignRequest));
    let old = h.open_card(D, Role::A).await.unwrap();
    let new = h.open_card(D, Role::A).await.unwrap();
    assert_eq!(request_state(&h, &old).await, SignRequestState::Superseded);
    let stale = h.card(D, old, CardAnswer::Sign(owner(Role::A))).await;
    assert_eq!(stale, Err(AppError::UnknownSignRequest));
    let out = h.card(D, new.clone(), CardAnswer::Expired).await;
    assert_eq!(out, Ok(SignOutcome::CardExpired { role: Role::A }));
    assert_eq!(request_state(&h, &new).await, SignRequestState::Expired);
}

#[tokio::test]
async fn cards_need_an_agreement_and_an_unsigned_side() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let early = h.open_card(D, Role::A).await;
    assert_eq!(early, Err(AppError::Domain(DomainError::NothingToSign)));
    let h = H::default();
    let hash = agreed(&h).await;
    h.sign(D, owner(Role::A), &hash.short()).await.unwrap();
    let again = h.open_card(D, Role::A).await;
    assert_eq!(again, Err(AppError::Domain(DomainError::AlreadySigned)));
}

#[tokio::test]
async fn ack_removes_outcome_from_undelivered() {
    let h = H::default();
    let request = RequestId("req-1".into());
    h.switch.push_outcome(CardOutcome {
        request: request.clone(),
        answer: CardAnswer::Expired,
    });
    assert_eq!(h.switch.undelivered().await.unwrap().len(), 1);
    h.switch.ack_outcome(&request).await.unwrap();
    assert!(h.switch.undelivered().await.unwrap().is_empty());
}
