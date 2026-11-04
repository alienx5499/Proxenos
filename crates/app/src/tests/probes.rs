//! Probe rules: own desk room only, three per round, own-side verdicts only.

use proxenos_domain::event::DealEvent;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::party::Role;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::ports::DealReader;
use crate::queries::get_party_view::get_party_view;

const D: &str = "D-0007";

async fn probed_events(h: &H) -> usize {
    let events = h.repo.events(&id(D)).await.unwrap();
    events
        .iter()
        .filter(|e| matches!(e.event, DealEvent::Probed { .. }))
        .count()
}

#[tokio::test]
async fn budget_exhausts_then_resets_next_round() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let t = po(98_000, 400, 560, 35);
    for left in [2, 1, 0] {
        let out = h.probe(D, Role::A, desk(Role::A), t.clone()).await.unwrap();
        assert_eq!(out.probes_left, left);
    }
    let spent = h.probe(D, Role::A, desk(Role::A), t.clone()).await;
    assert_eq!(spent, Err(AppError::ProbeBudgetSpent));
    assert_eq!(
        get_party_view(&h.repo, &id(D), Role::A)
            .await
            .unwrap()
            .probes_left,
        0
    );
    assert_eq!(
        get_party_view(&h.repo, &id(D), Role::B)
            .await
            .unwrap()
            .probes_left,
        3
    );

    h.offer(D, Role::A, t.clone()).await.unwrap();
    let still_round_one = h.probe(D, Role::A, desk(Role::A), t.clone()).await;
    assert_eq!(still_round_one, Err(AppError::ProbeBudgetSpent));
    h.offer(D, Role::B, po(112_000, 400, 560, 40))
        .await
        .unwrap();
    assert_eq!(
        get_party_view(&h.repo, &id(D), Role::A)
            .await
            .unwrap()
            .probes_left,
        3
    );
    let out = h.probe(D, Role::A, desk(Role::A), t).await.unwrap();
    assert_eq!(out.probes_left, 2);
    assert_eq!(probed_events(&h).await, 4);
}

#[tokio::test]
async fn probe_outside_own_desk_is_refused_without_spending() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let t = po(98_000, 400, 560, 35);
    for room in [deal_room(), desk(Role::B)] {
        let out = h.probe(D, Role::A, room, t.clone()).await;
        assert_eq!(out, Err(AppError::ProbeOutsideDesk));
    }
    assert_eq!(probed_events(&h).await, 0);
    assert_eq!(
        get_party_view(&h.repo, &id(D), Role::A)
            .await
            .unwrap()
            .probes_left,
        3
    );
}

#[tokio::test]
async fn probe_checks_own_limits_only() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    // Within the buyer's limits, outside the seller's on price and payment.
    let out = h
        .probe(D, Role::A, desk(Role::A), po(90_000, 400, 560, 60))
        .await
        .unwrap();
    assert!(out.within);
    assert!(out.failing_fields.is_empty());
    let out = h
        .probe(D, Role::B, desk(Role::B), po(90_000, 400, 560, 60))
        .await
        .unwrap();
    assert!(!out.within);
    assert_eq!(
        out.failing_fields,
        vec![Field::PricePerKg, Field::PaymentNetDays]
    );
}

#[tokio::test]
async fn only_the_bound_negotiator_may_probe() {
    let h = H::default();
    h.ready(D, DealKind::PurchaseOrder).await;
    let cmd = crate::commands::probe_own_limits::ProbeOwnLimits {
        deal: id(D),
        sender: owner(Role::A),
        room: desk(Role::A),
        terms: po(98_000, 400, 560, 35),
    };
    let out =
        crate::commands::probe_own_limits::probe_own_limits(&h.repo, &h.vault, &h.clock, cmd).await;
    assert!(matches!(out, Err(AppError::Domain(_))));
}
