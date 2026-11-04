use super::fixtures::*;
use super::*;
use crate::kind::Field;
use crate::terms::fixtures::{lease, po};

const A: Role = Role::A;
const B: Role = Role::B;

fn agreed_hash(d: &Deal) -> TermsHash {
    match d.state() {
        DealState::Agreed(h) | DealState::PartlySigned(h) => h,
        s => panic!("not agreed: {s:?}"),
    }
}

#[test]
fn purchase_order_happy_path_to_bound() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    assert_eq!((d.state(), d.turn(), d.round()), (DealState::Open, A, 1));
    offer(&mut d, A, po(100_000, 400, 58, 45)).unwrap();
    assert_eq!((d.turn(), d.round()), (B, 1));
    let counter = po(110_000, 400, 58, 45);
    let ev = offer(&mut d, B, counter.clone()).unwrap();
    assert!(
        matches!(&ev[..], [DealEvent::OfferMade { offer }] if offer.id.to_string() == "O-2" && offer.round == 1)
    );
    assert_eq!((d.turn(), d.round()), (A, 2));

    let ev = accept(&mut d, A, "O-2").unwrap();
    let hash = hash_of(&d, &counter);
    assert_eq!(d.state(), DealState::Agreed(hash));
    assert_eq!(d.agreed_offer().unwrap().terms, counter);
    assert!(matches!(ev[..], [DealEvent::Accepted { by: A, hash: h, .. }] if h == hash));

    let ev = sign(&mut d, A, hash, owner(A), "m1").unwrap();
    assert_eq!((ev.len(), d.state()), (1, DealState::PartlySigned(hash)));
    let ev = sign(&mut d, B, hash, owner(B), "m2").unwrap();
    assert_eq!(d.state(), DealState::Bound(hash));
    assert!(
        matches!(ev[..], [DealEvent::Signed { .. }, DealEvent::Bound { hash: h }] if h == hash)
    );
    assert_eq!(d.signatures().len(), 2);
}

#[test]
fn lease_happy_path_seller_accepts_first_offer() {
    let mut d = ready(DealKind::Lease, 8);
    let terms = lease(2_800_000, 2, 35, 12);
    offer(&mut d, A, terms.clone()).unwrap();
    accept(&mut d, B, "O-1").unwrap();
    let hash = hash_of(&d, &terms);
    sign(&mut d, B, hash, owner(B), "m1").unwrap();
    sign(&mut d, A, hash, owner(A), "m2").unwrap();
    assert_eq!(d.state(), DealState::Bound(hash));
}

#[test]
fn open_rejects_zero_rounds_and_emits_opened() {
    let id: DealId = "D-0009".parse().unwrap();
    let r = Deal::open(id.clone(), DealKind::Lease, room("r"), DEADLINE, 0, A);
    assert_eq!(r.unwrap_err(), DomainError::InvalidMaxRounds);
    let (d, ev) = Deal::open(
        id,
        DealKind::Lease,
        room("r"),
        DEADLINE,
        DEFAULT_MAX_ROUNDS,
        B,
    )
    .unwrap();
    assert_eq!(
        (d.state(), d.opener(), d.turn()),
        (DealState::AwaitingSeals, B, B)
    );
    assert!(matches!(
        ev[..],
        [DealEvent::Opened {
            max_rounds: 8,
            opener: B,
            ..
        }]
    ));
}

#[test]
fn offers_wait_for_both_seals() {
    let mut d = bind_all(new_deal(DealKind::PurchaseOrder, 8));
    let terms = po(110_000, 400, 58, 45);
    assert_eq!(
        offer(&mut d, A, terms.clone()),
        Err(DomainError::AwaitingSeals)
    );
    d.seal_limits(&po_limits(A)).unwrap();
    assert_eq!(d.state(), DealState::AwaitingSeals);
    assert_eq!(
        offer(&mut d, A, terms.clone()),
        Err(DomainError::AwaitingSeals)
    );
    d.seal_limits(&po_limits(B)).unwrap();
    assert_eq!(d.state(), DealState::Open);
    assert!(offer(&mut d, A, terms).is_ok());
}

#[test]
fn offer_only_on_your_turn() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let terms = po(110_000, 400, 58, 45);
    assert_eq!(
        offer(&mut d, B, terms.clone()),
        Err(DomainError::NotYourTurn)
    );
    offer(&mut d, A, terms.clone()).unwrap();
    assert_eq!(offer(&mut d, A, terms), Err(DomainError::NotYourTurn));
}

#[test]
fn offer_outside_own_limits_is_refused_and_changes_nothing() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let before = d.clone();
    let r = offer(&mut d, A, po(120_000, 400, 70, 45));
    let fields = vec![Field::PricePerKg, Field::DeliverBy];
    assert_eq!(r, Err(DomainError::OutsideOwnLimits(fields)));
    assert_eq!(d, before);
}

#[test]
fn offer_needs_own_limits_and_matching_kind() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let terms = po(110_000, 400, 58, 45);
    let wrong_role = po_limits(B);
    let r = d.submit_offer(
        &agent(A),
        terms.clone(),
        Note::default(),
        msg("o1"),
        &wrong_role,
        NOW,
    );
    assert_eq!(r, Err(DomainError::WrongLimitSet));
    let wrong_kind = lease_limits(A);
    let r = d.submit_offer(
        &agent(A),
        terms,
        Note::default(),
        msg("o1"),
        &wrong_kind,
        NOW,
    );
    assert_eq!(r, Err(DomainError::WrongLimitSet));
    let r = offer(&mut d, A, lease(2_800_000, 2, 35, 12));
    assert_eq!(r, Err(DomainError::KindMismatch));
}

#[test]
fn accept_only_the_counterpartys_latest_offer() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let terms = po(110_000, 400, 58, 45);
    assert_eq!(accept(&mut d, B, "O-1"), Err(DomainError::NotLatestOffer));
    offer(&mut d, A, terms.clone()).unwrap();
    assert_eq!(accept(&mut d, A, "O-1"), Err(DomainError::NotLatestOffer));
    assert_eq!(accept(&mut d, B, "O-9"), Err(DomainError::NotLatestOffer));
    offer(&mut d, B, terms).unwrap();
    assert_eq!(accept(&mut d, B, "O-1"), Err(DomainError::NotLatestOffer));
    assert_eq!(accept(&mut d, A, "O-1"), Err(DomainError::NotLatestOffer));
    assert!(accept(&mut d, A, "O-2").is_ok());
}

#[test]
fn accept_outside_own_limits_is_refused() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    offer(&mut d, A, po(100_000, 400, 58, 45)).unwrap();
    let before = d.clone();
    let r = accept(&mut d, B, "O-1");
    assert_eq!(
        r,
        Err(DomainError::OutsideOwnLimits(vec![Field::PricePerKg]))
    );
    assert_eq!(d, before);
    let r = d.accept(&agent(B), "O-1".parse().unwrap(), &po_limits(A), NOW);
    assert_eq!(r, Err(DomainError::WrongLimitSet));
}

#[test]
fn agreed_terms_are_frozen() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let terms = po(110_000, 400, 58, 45);
    offer(&mut d, A, terms.clone()).unwrap();
    accept(&mut d, B, "O-1").unwrap();
    let r = offer(&mut d, B, po(111_000, 400, 58, 45));
    assert_eq!(r, Err(DomainError::DealClosed));
    assert_eq!(accept(&mut d, A, "O-1"), Err(DomainError::DealClosed));
    assert_eq!(d.agreed_offer().unwrap().terms, terms);
    assert_eq!(d.offers().len(), 1);
}

fn agreed_po() -> Deal {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    offer(&mut d, A, po(110_000, 400, 58, 45)).unwrap();
    accept(&mut d, B, "O-1").unwrap();
    d
}

#[test]
fn sign_before_agreement_is_rejected() {
    let mut d = ready(DealKind::PurchaseOrder, 8);
    let h = hash_of(&d, &po(110_000, 400, 58, 45));
    let r = sign(&mut d, A, h, owner(A), "m1");
    assert_eq!(r, Err(DomainError::NothingToSign));
}

#[test]
fn signature_must_match_hash() {
    let mut d = agreed_po();
    let stale = hash_of(&d, &po(109_000, 400, 58, 45));
    let r = sign(&mut d, A, stale, owner(A), "m1");
    assert_eq!(r, Err(DomainError::HashMismatch));
}

#[test]
fn signature_must_come_from_bound_owner() {
    let mut d = agreed_po();
    let h = agreed_hash(&d);
    for signer in [owner(B), agent(A), pid("look-alike")] {
        let r = sign(&mut d, A, h, signer, "m1");
        assert_eq!(r, Err(DomainError::NotBoundOwner));
    }
    assert!(d.signatures().is_empty());
}

#[test]
fn signature_once_per_side_and_message() {
    let mut d = agreed_po();
    let h = agreed_hash(&d);
    sign(&mut d, A, h, owner(A), "m1").unwrap();
    let r = sign(&mut d, A, h, owner(A), "m2");
    assert_eq!(r, Err(DomainError::AlreadySigned));
    let r = sign(&mut d, B, h, owner(B), "m1");
    assert_eq!(r, Err(DomainError::DuplicateMessage));
    assert_eq!(d.state(), DealState::PartlySigned(h));
}

#[test]
fn signature_after_deadline_or_bound_is_rejected() {
    let mut d = agreed_po();
    let h = agreed_hash(&d);
    let late = sign_at(&mut d, A, h, owner(A), "m1", DEADLINE);
    assert_eq!(late, Err(DomainError::DeadlinePassed));
    sign(&mut d, A, h, owner(A), "m1").unwrap();
    sign(&mut d, B, h, owner(B), "m2").unwrap();
    let r = sign(&mut d, B, h, owner(B), "m3");
    assert_eq!(r, Err(DomainError::DealClosed));
}

#[test]
fn seller_opener_reaches_the_demo_fixture_state() {
    let mut d = bind_and_seal(new_deal_as("D-0008", DealKind::PurchaseOrder, 8, B));
    assert_eq!(
        offer(&mut d, A, po(110_000, 400, 58, 45)),
        Err(DomainError::NotYourTurn)
    );
    for by in [B, A, B, A, B] {
        offer(&mut d, by, po(110_000, 400, 58, 45)).unwrap();
    }
    let last = d.latest_offer().unwrap();
    assert_eq!(
        (last.id.to_string(), last.by, last.round),
        ("O-5".into(), B, 3)
    );
    assert_eq!((d.turn(), d.round()), (A, 3));
    assert!(accept(&mut d, A, "O-5").is_ok());
}

#[test]
fn same_terms_in_two_deals_hash_differently() {
    let terms = po(110_000, 400, 58, 45);
    let mut hashes = Vec::new();
    for id in ["D-0007", "D-0009"] {
        let mut d = bind_and_seal(new_deal_as(id, DealKind::PurchaseOrder, 8, A));
        offer(&mut d, A, terms.clone()).unwrap();
        accept(&mut d, B, "O-1").unwrap();
        assert_eq!(agreed_hash(&d), hash_of(&d, &terms));
        hashes.push(agreed_hash(&d));
    }
    assert_ne!(hashes[0], hashes[1]);
}

#[path = "deal_tests_identity.rs"]
mod identity;
#[path = "deal_tests_lifecycle.rs"]
mod lifecycle;
#[path = "deal_tests_record.rs"]
mod record;
