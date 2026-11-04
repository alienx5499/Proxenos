//! Round cap, deadline, withdrawal, setup and the no-limit-values log.

use super::super::fixtures::*;
use super::super::*;
use super::{A, B};
use crate::event::ExpiryReason;
use crate::terms::fixtures::po;

fn play_rounds(d: &mut Deal, rounds: u16) {
    for _ in 0..rounds {
        offer(d, A, po(110_000, 400, 58, 45)).unwrap();
        offer(d, B, po(110_000, 400, 58, 45)).unwrap();
    }
}

#[test]
fn round_cap_expires_on_next_offer() {
    let mut d = ready(DealKind::PurchaseOrder, 2);
    play_rounds(&mut d, 2);
    let rounds: Vec<u16> = d.offers().iter().map(|o| o.round).collect();
    assert_eq!(rounds, [1, 1, 2, 2]);
    assert_eq!((d.round(), d.turn()), (2, A));
    let ev = offer(&mut d, A, po(110_000, 400, 58, 45)).unwrap();
    assert_eq!(d.state(), DealState::Expired);
    let reason = ExpiryReason::RoundCap;
    assert_eq!(ev, vec![DealEvent::Expired { reason }]);
    assert_eq!(d.offers().len(), 4);
}

#[test]
fn final_offer_can_still_be_accepted_at_round_cap() {
    let mut d = ready(DealKind::PurchaseOrder, 1);
    play_rounds(&mut d, 1);
    assert!(accept(&mut d, A, "O-2").is_ok());
}

#[test]
fn deadline_expires_and_blocks_moves() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    assert!(d.expire_due(Timestamp(DEADLINE.0 - 1)).is_empty());
    let terms = po(110_000, 400, 58, 45);
    let late = d.submit_offer(
        &agent(A),
        terms,
        Note::default(),
        msg("o1"),
        &po_limits(A),
        DEADLINE,
    );
    assert_eq!(late, Err(DomainError::DeadlinePassed));
    let reason = ExpiryReason::Deadline;
    assert_eq!(d.expire_due(DEADLINE), vec![DealEvent::Expired { reason }]);
    assert_eq!(d.state(), DealState::Expired);
    assert!(d.expire_due(Timestamp(DEADLINE.0 + 5)).is_empty());
    assert_eq!(d.withdraw(&owner(A)), Err(DomainError::DealClosed));
}

#[test]
fn expiry_stops_a_partly_signed_deal_but_not_a_bound_one() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    offer(&mut d, A, po(110_000, 400, 58, 45)).unwrap();
    accept(&mut d, B, "O-1").unwrap();
    let DealState::Agreed(h) = d.state() else {
        panic!()
    };
    sign(&mut d, A, h, owner(A), "m1").unwrap();
    let mut bound = d.clone();
    sign(&mut bound, B, h, owner(B), "m2").unwrap();
    assert!(bound.expire_due(DEADLINE).is_empty());
    assert_eq!(d.expire_due(DEADLINE).len(), 1);
    let r = sign(&mut d, B, h, owner(B), "m2");
    assert_eq!(r, Err(DomainError::DealClosed));
}

#[test]
fn withdraw_by_bound_owner_before_bound() {
    let mut d = ready(DealKind::Lease, 8);
    for not_owner in [agent(B), pid("stranger")] {
        assert_eq!(d.withdraw(&not_owner), Err(DomainError::NotBoundOwner));
    }
    let ev = d.withdraw(&owner(B)).unwrap();
    assert_eq!(ev, vec![DealEvent::Withdrawn { by: B }]);
    assert_eq!(d.state(), DealState::Withdrawn);
    assert_eq!(d.withdraw(&owner(A)), Err(DomainError::DealClosed));
    let r = offer(
        &mut d,
        A,
        crate::terms::fixtures::lease(2_800_000, 2, 35, 12),
    );
    assert_eq!(r, Err(DomainError::DealClosed));
}

#[test]
fn desk_and_party_binding_rules() {
    let mut d = new_deal(DealKind::PurchaseOrder, 8);
    let r = d.bind_party(A, owner(A), &desk(A));
    assert_eq!(r, Err(DomainError::DeskNotRecorded));
    d.record_desk(A, desk(A)).unwrap();
    assert_eq!(
        d.record_desk(A, room("other")),
        Err(DomainError::DeskAlreadyRecorded)
    );
    let r = d.bind_negotiator(A, agent(A), &desk(A));
    assert_eq!(r, Err(DomainError::PartyNotBound));
    assert_eq!(
        d.bind_party(A, owner(A), &room("deal-room")),
        Err(DomainError::WrongRoom)
    );
    d.bind_party(A, owner(A), &desk(A)).unwrap();
    let r = d.bind_party(A, pid("late"), &desk(A));
    assert_eq!(r, Err(DomainError::AlreadyBound));
    let r = d.bind_negotiator(A, owner(A), &desk(A));
    assert_eq!(r, Err(DomainError::OwnerIsNegotiator));
    let r = d.bind_negotiator(A, agent(A), &room("deal-room"));
    assert_eq!(r, Err(DomainError::WrongRoom));
    d.bind_negotiator(A, agent(A), &desk(A)).unwrap();
    let r = d.bind_negotiator(A, pid("spare"), &desk(A));
    assert_eq!(r, Err(DomainError::NegotiatorAlreadyBound));
    let b = d.binding(A).unwrap();
    assert_eq!((&b.owner, b.agent.as_ref()), (&owner(A), Some(&agent(A))));
}

#[test]
fn seal_rules() {
    let mut d = new_deal(DealKind::PurchaseOrder, 8);
    let r = d.seal_limits(&lease_limits(A));
    assert_eq!(r, Err(DomainError::WrongLimitSet));
    d.seal_limits(&po_limits(A)).unwrap();
    assert!(d.is_sealed(A) && !d.is_sealed(B));
    let r = d.seal_limits(&po_limits(A));
    assert_eq!(r, Err(DomainError::AlreadySealed));
}

#[test]
fn log_never_carries_limit_values() {
    let mut log = Vec::new();
    let (mut d, ev) = Deal::open(
        "D-0007".parse().unwrap(),
        DealKind::PurchaseOrder,
        room("deal-room"),
        DEADLINE,
        8,
        A,
    )
    .unwrap();
    log.extend(ev);
    for role in [A, B] {
        log.extend(d.record_desk(role, desk(role)).unwrap());
        log.extend(d.bind_party(role, owner(role), &desk(role)).unwrap());
        log.extend(d.bind_negotiator(role, agent(role), &desk(role)).unwrap());
        log.extend(d.seal_limits(&po_limits(role)).unwrap());
    }
    log.extend(offer(&mut d, A, po(100_000, 400, 58, 45)).unwrap());
    log.extend(offer(&mut d, B, po(110_000, 400, 58, 45)).unwrap());
    log.extend(accept(&mut d, A, "O-2").unwrap());
    let DealState::Agreed(h) = d.state() else {
        panic!()
    };
    log.extend(sign(&mut d, A, h, owner(A), "m1").unwrap());
    log.extend(sign(&mut d, B, h, owner(B), "m2").unwrap());
    let json = serde_json::to_string(&log).unwrap();
    for sealed in ["115000", "102000"] {
        assert!(!json.contains(sealed), "{sealed} leaked");
    }
    let back: Vec<DealEvent> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, log);
    let deal_json = serde_json::to_string(&d).unwrap();
    assert_eq!(serde_json::from_str::<Deal>(&deal_json).unwrap(), d);
}
