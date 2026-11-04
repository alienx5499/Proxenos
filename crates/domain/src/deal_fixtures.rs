//! Test fixtures: sealed limits for both kinds and a deal ready for offers.

use std::collections::BTreeMap;

use super::*;
use crate::date::Date;
use crate::kind::Field;
use crate::limit::Constraint;
pub use crate::limit::LimitSet;
use crate::money::{Currency, Money};
pub use crate::offer::Note;
use crate::party::ParticipantId;
use crate::quantity::Quantity;
use crate::signature::MessageId;
use crate::terms::FieldValue;

pub const NOW: Timestamp = Timestamp(1_000);
pub const DEADLINE: Timestamp = Timestamp(10_000);

fn inr(minor: i64) -> FieldValue {
    FieldValue::Money(Money::new(minor, Currency::Inr))
}

fn int(n: u32) -> FieldValue {
    FieldValue::Integer(n)
}

fn set(kind: DealKind, role: Role, limits: [(Field, Constraint); 4]) -> LimitSet {
    LimitSet::new(kind, role, BTreeMap::from(limits)).unwrap()
}

/// Buyer: price <= 115000, 300..=500 kg, by day 60, net >= 30.
/// Seller: price >= 102000, 200..=600 kg, from day 50, net <= 45.
pub fn po_limits(role: Role) -> LimitSet {
    use Constraint::*;
    let kg = |v| FieldValue::Quantity(Quantity::kg(v));
    let qty = (Field::QuantityKg, Between(kg(300), kg(500)));
    let limits = match role {
        Role::A => [
            (Field::PricePerKg, AtMost(inr(115_000))),
            qty,
            (Field::DeliverBy, NoLaterThan(Date::from_days(60))),
            (Field::PaymentNetDays, AtLeast(int(30))),
        ],
        Role::B => [
            (Field::PricePerKg, AtLeast(inr(102_000))),
            (Field::QuantityKg, Between(kg(200), kg(600))),
            (Field::DeliverBy, NoEarlierThan(Date::from_days(50))),
            (Field::PaymentNetDays, AtMost(int(45))),
        ],
    };
    set(DealKind::PurchaseOrder, role, limits)
}

/// Tenant: rent <= 3000000, deposit <= 3, move in by day 40, 6..=12 months.
/// Landlord: rent >= 2500000, deposit >= 2, from day 30, 11..=24 months.
pub fn lease_limits(role: Role) -> LimitSet {
    use Constraint::*;
    let limits = match role {
        Role::A => [
            (Field::MonthlyRent, AtMost(inr(3_000_000))),
            (Field::DepositMonths, AtMost(int(3))),
            (Field::MoveInBy, NoLaterThan(Date::from_days(40))),
            (Field::TermMonths, Between(int(6), int(12))),
        ],
        Role::B => [
            (Field::MonthlyRent, AtLeast(inr(2_500_000))),
            (Field::DepositMonths, AtLeast(int(2))),
            (Field::MoveInBy, NoEarlierThan(Date::from_days(30))),
            (Field::TermMonths, Between(int(11), int(24))),
        ],
    };
    set(DealKind::Lease, role, limits)
}

pub fn limits(kind: DealKind, role: Role) -> LimitSet {
    match kind {
        DealKind::PurchaseOrder => po_limits(role),
        DealKind::Lease => lease_limits(role),
    }
}

pub fn pid(s: &str) -> ParticipantId {
    ParticipantId::new(s).unwrap()
}

pub fn room(s: &str) -> RoomId {
    RoomId::new(s).unwrap()
}

pub fn desk(role: Role) -> RoomId {
    room(&format!("desk-{role:?}"))
}

pub fn owner(role: Role) -> ParticipantId {
    pid(&format!("owner-{role:?}"))
}

pub fn agent(role: Role) -> ParticipantId {
    pid(&format!("agent-{role:?}"))
}

pub fn new_deal_as(id: &str, kind: DealKind, max_rounds: u16, opener: Role) -> Deal {
    let id = id.parse().unwrap();
    Deal::open(id, kind, room("deal-room"), DEADLINE, max_rounds, opener)
        .unwrap()
        .0
}

pub fn new_deal(kind: DealKind, max_rounds: u16) -> Deal {
    new_deal_as("D-0007", kind, max_rounds, DEFAULT_OPENER)
}

/// Desks recorded, both owners and negotiators bound, both sides sealed: Open.
pub fn ready(kind: DealKind, max_rounds: u16) -> Deal {
    bind_and_seal(new_deal(kind, max_rounds))
}

pub fn bind_all(mut d: Deal) -> Deal {
    for role in [Role::A, Role::B] {
        d.record_desk(role, desk(role)).unwrap();
        d.bind_party(role, owner(role), &desk(role)).unwrap();
        d.bind_negotiator(role, agent(role), &desk(role)).unwrap();
    }
    d
}

pub fn bind_and_seal(d: Deal) -> Deal {
    let mut d = bind_all(d);
    for role in [Role::A, Role::B] {
        d.seal_limits(&limits(d.kind(), role)).unwrap();
    }
    d
}

/// The hash this deal's fixture owners produce for `terms`.
pub fn hash_of(d: &Deal, terms: &Terms) -> TermsHash {
    TermsHash::of(terms, d.id(), [&owner(Role::A), &owner(Role::B)])
}

pub fn msg(s: &str) -> MessageId {
    MessageId::new(s).unwrap()
}

pub fn offer(d: &mut Deal, by: Role, terms: Terms) -> Result<Vec<DealEvent>, DomainError> {
    let own = limits(d.kind(), by);
    let id = msg(&format!("offer-{}", d.offers().len() + 1));
    d.submit_offer(&agent(by), terms, Note::default(), id, &own, NOW)
}

pub fn accept(d: &mut Deal, by: Role, id: &str) -> Result<Vec<DealEvent>, DomainError> {
    let own = limits(d.kind(), by);
    d.accept(&agent(by), id.parse().unwrap(), &own, NOW)
}

pub fn sign_at(
    d: &mut Deal,
    role: Role,
    hash: TermsHash,
    signer: ParticipantId,
    id: &str,
    now: Timestamp,
) -> Result<Vec<DealEvent>, DomainError> {
    d.record_signature(role, hash, msg(id), signer, now)
}

pub fn sign(
    d: &mut Deal,
    role: Role,
    hash: TermsHash,
    signer: ParticipantId,
    id: &str,
) -> Result<Vec<DealEvent>, DomainError> {
    sign_at(d, role, hash, signer, id, NOW)
}
