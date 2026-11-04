//! Join codes: single use, room-scoped, normalized, capped, unique at issue; up-front bindings.

use std::sync::atomic::Ordering;

use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::commands::open_deal::{OpenDeal, RecordDesk, open_deal, record_desk};
use crate::commands::record_deal_sheet::record_deal_sheet;
use crate::commands::seal_limits::Sealed;
use crate::ports::{DealReader, DocId};
use crate::secret::normalize_join_code;
use crate::stored::{JOIN_ATTEMPTS, PendingBinding};

const D: &str = "D-0007";

async fn sealed_a(h: &H, deal: &str) -> Sealed {
    let tokens = h.open(deal, DealKind::PurchaseOrder).await;
    h.seal(&tokens.role_a, limits(DealKind::PurchaseOrder, Role::A))
        .await
        .unwrap()
}

#[tokio::test]
async fn join_code_is_single_use_and_normalized() {
    let h = H::default();
    let code = sealed_a(&h, D).await.join_code;
    assert_eq!(code.len(), 9);
    assert_eq!(&code[4..5], "-");
    let wrong_room = h.join(&code, owner(Role::A), deal_room()).await;
    assert_eq!(wrong_room, Err(AppError::UnknownJoinCode));
    let typed = normalize_join_code(&code).to_ascii_lowercase();
    assert_eq!(
        h.join(&typed, owner(Role::A), desk(Role::A)).await,
        Ok((id(D), Role::A))
    );
    let replay = h.join(&code, agent(Role::A), desk(Role::A)).await;
    assert_eq!(replay, Err(AppError::UnknownJoinCode));
}

#[tokio::test]
async fn wrong_codes_lock_the_side() {
    let h = H::default();
    let code = sealed_a(&h, D).await.join_code;
    for i in 1..=JOIN_ATTEMPTS {
        let out = h.join("AAAA-AAAA", pid("guesser"), desk(Role::A)).await;
        assert_eq!(
            out,
            Err(AppError::WrongJoinCode {
                attempts_left: JOIN_ATTEMPTS - i
            })
        );
    }
    let locked = h.join(&code, owner(Role::A), desk(Role::A)).await;
    assert_eq!(locked, Err(AppError::JoinLocked));
}

#[tokio::test]
async fn join_codes_are_unique_among_pending_codes() {
    let h = H::default();
    let po_a = || limits(DealKind::PurchaseOrder, Role::A);
    let first = h.open(D, DealKind::PurchaseOrder).await;
    let rooms = [room("desk-a2"), room("desk-b2")];
    let second = h
        .open_at("D-0008", DealKind::PurchaseOrder, room("room-2"), rooms)
        .await;
    h.secrets.stuck.store(true, Ordering::SeqCst);
    h.seal(&first.role_a, po_a()).await.unwrap();
    let clash = h.seal(&second.role_a, po_a()).await;
    assert_eq!(clash.err(), Some(AppError::Internal("no unused join code")));
}

#[tokio::test]
async fn parties_named_at_open_bind_in_the_seal_save() {
    let h = H::default();
    let cmd = OpenDeal {
        id: id(D),
        kind: DealKind::PurchaseOrder,
        room: deal_room(),
        deadline: DEADLINE,
        max_rounds: 8,
        opener: Role::B,
        parties: Some([Role::A, Role::B].map(|r| PendingBinding {
            owner: owner(r),
            agent: agent(r),
        })),
    };
    let tokens = open_deal(&h.repo, &h.clock, &h.secrets, cmd).await.unwrap();
    for role in [Role::A, Role::B] {
        let cmd = RecordDesk {
            deal: id(D),
            role,
            room: desk(role),
        };
        record_desk(&h.repo, &h.clock, cmd).await.unwrap();
    }
    let sealed = h
        .seal(&tokens.role_a, limits(DealKind::PurchaseOrder, Role::A))
        .await
        .unwrap();
    assert!(sealed.bound);
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert_eq!(stored.deal.opener(), Role::B);
    let binding = stored.deal.binding(Role::A).unwrap();
    assert_eq!(binding.owner, owner(Role::A));
    assert_eq!(binding.agent, Some(agent(Role::A)));
    let party = stored.party(Role::A);
    assert_eq!((party.join_code, &party.pending_binding), (None, &None));
    assert!(stored.party(Role::B).pending_binding.is_some());
    let spent = h
        .join(&sealed.join_code, owner(Role::A), desk(Role::A))
        .await;
    assert_eq!(spent, Err(AppError::UnknownJoinCode));
}

#[tokio::test]
async fn deal_sheet_doc_is_recorded() {
    let h = H::default();
    h.open(D, DealKind::Lease).await;
    record_deal_sheet(&h.repo, &h.clock, &id(D), &DocId("doc-9".into()))
        .await
        .unwrap();
    let stored = h.repo.load(&id(D)).await.unwrap().unwrap();
    assert_eq!(stored.deal_sheet_doc.as_deref(), Some("doc-9"));
}
