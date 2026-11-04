//! Who may bind, move, sign and reuse message ids.

use super::super::fixtures::*;
use super::super::*;
use super::{A, B, agreed_po};
use crate::terms::fixtures::po;

fn desks_recorded() -> Deal {
    let mut d = new_deal(DealKind::PurchaseOrder, 8);
    d.record_desk(A, desk(A)).unwrap();
    d.record_desk(B, desk(B)).unwrap();
    d
}

#[test]
fn one_person_cannot_own_both_sides() {
    let mut d = desks_recorded();
    d.bind_party(A, owner(A), &desk(A)).unwrap();
    let r = d.bind_party(B, owner(A), &desk(B));
    assert_eq!(r, Err(DomainError::OwnerIsCounterparty));
    assert!(d.binding(B).is_none());
}

#[test]
fn owners_and_negotiators_are_all_distinct() {
    let mut d = desks_recorded();
    d.bind_party(A, owner(A), &desk(A)).unwrap();
    d.bind_negotiator(A, agent(A), &desk(A)).unwrap();
    let r = d.bind_party(B, agent(A), &desk(B));
    assert_eq!(r, Err(DomainError::OwnerIsNegotiator));
    d.bind_party(B, owner(B), &desk(B)).unwrap();
    let r = d.bind_negotiator(B, owner(A), &desk(B));
    assert_eq!(r, Err(DomainError::OwnerIsNegotiator));
    let r = d.bind_negotiator(B, owner(B), &desk(B));
    assert_eq!(r, Err(DomainError::OwnerIsNegotiator));
    let r = d.bind_negotiator(B, agent(A), &desk(B));
    assert_eq!(r, Err(DomainError::NegotiatorIsCounterparty));
    assert!(d.bind_negotiator(B, agent(B), &desk(B)).is_ok());
}

#[test]
fn negotiator_binding_closes_at_agreement() {
    let mut d = agreed_po();
    let r = d.bind_negotiator(A, pid("spare"), &desk(A));
    assert_eq!(r, Err(DomainError::DealClosed));
}

#[test]
fn only_the_bound_negotiator_moves() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let own = po_limits(A);
    for actor in [owner(A), pid("stranger")] {
        let terms = po(110_000, 400, 58, 45);
        let r = d.submit_offer(&actor, terms, Note::default(), msg("o1"), &own, NOW);
        assert_eq!(r, Err(DomainError::NotBoundNegotiator));
    }
    offer(&mut d, A, po(110_000, 400, 58, 45)).unwrap();
    let r = d.accept(&owner(B), "O-1".parse().unwrap(), &po_limits(B), NOW);
    assert_eq!(r, Err(DomainError::NotBoundNegotiator));
    assert_eq!(d.state(), DealState::Open);
}

#[test]
fn message_ids_are_single_use_across_offers_and_signatures() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    offer(&mut d, A, po(110_000, 400, 58, 45)).unwrap();
    let terms = po(110_000, 400, 58, 45);
    let r = d.submit_offer(
        &agent(B),
        terms,
        Note::default(),
        msg("offer-1"),
        &po_limits(B),
        NOW,
    );
    assert_eq!(r, Err(DomainError::DuplicateMessage));
    accept(&mut d, B, "O-1").unwrap();
    let DealState::Agreed(h) = d.state() else {
        panic!()
    };
    let r = sign(&mut d, A, h, owner(A), "offer-1");
    assert_eq!(r, Err(DomainError::DuplicateMessage));
}

#[test]
fn signature_time_is_the_notary_clock() {
    let mut d = agreed_po();
    let DealState::Agreed(h) = d.state() else {
        panic!()
    };
    sign_at(&mut d, A, h, owner(A), "m1", Timestamp(2_000)).unwrap();
    assert_eq!(d.signatures()[0].at, Timestamp(2_000));
}
