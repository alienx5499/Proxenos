//! Reopen, desks, negotiator exclusion, listing, admit, withdraw, expiry.

use proxenos_domain::deal::DEFAULT_OPENER;
use proxenos_domain::error::DomainError;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::commands::admit_parties::{AdmitParties, admit_parties};
use crate::commands::bind_negotiator::{BindNegotiator, bind_negotiator};
use crate::commands::expire_due::expire_due;
use crate::commands::open_deal::{OpenDeal, RecordDesk, open_deal, open_deal_in, record_desk};
use crate::commands::withdraw::{Withdraw, withdraw};
use crate::fakes::{Admitted, FakeAdmin};
use crate::ports::DealReader;
use crate::queries::list_deals::list_deals;

const D: &str = "D-0007";

#[tokio::test]
async fn reopen_is_refused() {
    let h = H::default();
    h.open(D, DealKind::PurchaseOrder).await;
    let again = OpenDeal {
        id: id(D),
        kind: DealKind::Lease,
        room: deal_room(),
        deadline: DEADLINE,
        max_rounds: 8,
        opener: DEFAULT_OPENER,
        parties: None,
    };
    let err = open_deal(&h.repo, &h.clock, &h.secrets, again).await.err();
    assert_eq!(err, Some(AppError::DealExists(id(D))));
}

#[tokio::test]
async fn desk_room_must_be_distinct() {
    let h = H::default();
    let cmd = OpenDeal {
        id: id(D),
        kind: DealKind::Lease,
        room: deal_room(),
        deadline: DEADLINE,
        max_rounds: 8,
        opener: DEFAULT_OPENER,
        parties: None,
    };
    open_deal(&h.repo, &h.clock, &h.secrets, cmd).await.unwrap();
    let desk_cmd = |role, room| RecordDesk {
        deal: id(D),
        role,
        room,
    };
    let wrong = Err(AppError::Domain(DomainError::WrongRoom));
    assert_eq!(
        record_desk(&h.repo, &h.clock, desk_cmd(Role::A, deal_room())).await,
        wrong
    );
    record_desk(&h.repo, &h.clock, desk_cmd(Role::A, desk(Role::A)))
        .await
        .unwrap();
    assert_eq!(
        record_desk(&h.repo, &h.clock, desk_cmd(Role::B, desk(Role::A))).await,
        wrong
    );
}

#[tokio::test]
async fn excluded_ids_cannot_become_negotiators() {
    let h = H::default();
    let tokens = h.open(D, DealKind::PurchaseOrder).await;
    let sealed = h
        .seal(&tokens.role_a, limits(DealKind::PurchaseOrder, Role::A))
        .await
        .unwrap();
    h.join(&sealed.join_code, owner(Role::A), desk(Role::A))
        .await
        .unwrap();
    let cmd = BindNegotiator {
        deal: id(D),
        sender: pid("relay"),
        room: desk(Role::A),
        excluded: vec![pid("notary"), pid("relay")],
    };
    assert_eq!(
        bind_negotiator(&h.repo, &h.clock, cmd).await,
        Err(AppError::ExcludedNegotiator)
    );
}

#[tokio::test]
async fn list_deals_by_room() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let listed = list_deals(&h.repo, &desk(Role::B)).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].desk_of, Some(Role::B));
    assert_eq!(
        list_deals(&h.repo, &deal_room()).await.unwrap()[0].desk_of,
        None
    );
}

#[tokio::test]
async fn admit_validates_and_is_idempotent() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let admin = FakeAdmin {
        groups: [("D-0007 cafe".to_owned(), "g1".to_owned())].into(),
        users: [
            ("asha".to_owned(), "u1".to_owned()),
            ("ravi".to_owned(), "u2".to_owned()),
        ]
        .into(),
        ..Default::default()
    };
    let cmd = |users: Vec<&str>| AdmitParties {
        deal: id(D),
        group_name: "D-0007 cafe".into(),
        owner_usernames: users.into_iter().map(Into::into).collect(),
        agent_ids: vec!["a1".into(), "a2".into()],
    };
    let short = admit_parties(&h.repo, &admin, &h.clock, cmd(vec!["asha"])).await;
    assert_eq!(short, Err(AppError::AdmitNeedsTwoEach));
    for _ in 0..2 {
        let group = admit_parties(&h.repo, &admin, &h.clock, cmd(vec!["asha", "ravi"])).await;
        assert_eq!(group.as_deref(), Ok("g1"));
    }
    let expected = Admitted {
        group: "g1".into(),
        users: vec!["u1".into(), "u2".into()],
        agents: vec!["a1".into(), "a2".into()],
    };
    assert_eq!(*admin.admitted.lock().unwrap(), vec![expected]);
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert_eq!(stored.group_id.as_deref(), Some("g1"));
}

#[tokio::test]
async fn withdraw_by_bound_owner_only() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let stranger = withdraw(
        &h.repo,
        &h.clock,
        Withdraw {
            deal: id(D),
            sender: agent(Role::A),
        },
    )
    .await;
    assert_eq!(stranger, Err(AppError::Domain(DomainError::NotBoundOwner)));
    let by = withdraw(
        &h.repo,
        &h.clock,
        Withdraw {
            deal: id(D),
            sender: owner(Role::B),
        },
    )
    .await;
    assert_eq!(by, Ok(Role::B));
}

#[tokio::test]
async fn expire_due_expires_past_deadline() {
    let h = H::default();
    h.ready("D-0008", DealKind::Lease).await;
    let report = expire_due(&h.repo, &h.clock).await.unwrap();
    assert!(report.expired.is_empty() && report.failed.is_empty());
    h.clock.set(Timestamp(DEADLINE.0));
    let report = expire_due(&h.repo, &h.clock).await.unwrap();
    assert_eq!(report.expired, vec![id("D-0008")]);
    let late = h
        .offer("D-0008", Role::A, lease(2_600_000, 2, 35, 12))
        .await;
    assert_eq!(late, Err(AppError::Domain(DomainError::DealClosed)));
}

#[tokio::test]
async fn rooms_of_a_live_deal_cannot_be_reused_and_desks_record_at_open() {
    let h = H::default();
    let open = |deal: &str, room| OpenDeal {
        id: id(deal),
        kind: DealKind::PurchaseOrder,
        room,
        deadline: DEADLINE,
        max_rounds: 8,
        opener: DEFAULT_OPENER,
        parties: None,
    };
    let desks = [desk(Role::A), desk(Role::B)];
    let s = &h.secrets;
    open_deal_in(
        &h.repo,
        &h.clock,
        s,
        open(D, deal_room()),
        Some(desks.clone()),
    )
    .await
    .unwrap();
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert_eq!(stored.deal.desk(Role::B), Some(&desk(Role::B)));
    assert_eq!(stored.version, 1, "desks recorded in the open save");
    let in_use = Some(AppError::RoomInUse(id(D)));
    let again = open_deal(&h.repo, &h.clock, s, open("D-0008", deal_room())).await;
    assert_eq!(again.err(), in_use);
    let reuse_desk = open_deal(&h.repo, &h.clock, s, open("D-0008", desk(Role::A))).await;
    assert_eq!(reuse_desk.err(), in_use);
    let other = [room("desk-x"), desk(Role::B)];
    let desk_clash = open_deal_in(
        &h.repo,
        &h.clock,
        s,
        open("D-0009", room("r9")),
        Some(other),
    );
    assert_eq!(desk_clash.await.err(), in_use);
}
