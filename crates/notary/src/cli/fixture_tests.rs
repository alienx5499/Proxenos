use proxenos_app::commands::open_deal::{OpenDeal, RecordDesk, open_deal, record_desk};
use proxenos_app::fakes::{
    CountingSecrets, FakeSwitch, FakeVault, FixedClock, InMemoryDeals, SwitchCall,
};
use proxenos_app::ports::DealReader;
use proxenos_domain::deal::DealState;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;

use super::fixture::{Plan, SELLER_LAST, scripted_limits, scripted_prices};
use super::fixture_play::{Ports, play};
use crate::config::Secret;

fn pid(s: &str) -> ParticipantId {
    ParticipantId::new(s).unwrap()
}

#[test]
fn prices_end_on_the_seller_at_the_scripted_number() {
    let seller_opens = scripted_prices(Role::B, 3);
    assert_eq!(seller_opens.len(), 5);
    assert_eq!(seller_opens.last(), Some(&(Role::B, SELLER_LAST)));
    assert_eq!(seller_opens[0], (Role::B, SELLER_LAST + 96));
    let buyer_opens = scripted_prices(Role::A, 3);
    assert_eq!(buyer_opens.len(), 4);
    assert_eq!(buyer_opens.last(), Some(&(Role::B, SELLER_LAST)));
    for (role, price) in buyer_opens.iter().chain(&seller_opens) {
        let terms = proxenos_domain::terms::FieldValue::Money(proxenos_domain::money::Money::new(
            *price,
            proxenos_domain::money::Currency::Usd,
        ));
        let own = scripted_limits(*role, 0)
            .get(proxenos_domain::kind::Field::PricePerKg)
            .unwrap();
        assert!(own.admits(terms), "{role:?} {price} outside its own limit");
    }
    assert!(scripted_prices(Role::A, 1).is_empty());
}

struct World {
    repo: InMemoryDeals,
    vault: FakeVault,
    clock: FixedClock,
    secrets: CountingSecrets,
    switch: FakeSwitch,
}

impl World {
    /// D-0008 opened by a kickoff with the seller opening, both desks recorded.
    async fn new() -> (Self, [String; 2]) {
        let w = World {
            repo: InMemoryDeals::default(),
            vault: FakeVault,
            clock: FixedClock::default(),
            secrets: CountingSecrets::default(),
            switch: FakeSwitch::default(),
        };
        w.clock.set(Timestamp(864_000));
        let cmd = OpenDeal {
            id: "D-0008".parse().unwrap(),
            kind: DealKind::PurchaseOrder,
            room: RoomId::new("deal").unwrap(),
            deadline: Timestamp(10_000_000),
            max_rounds: 8,
            opener: Role::B,
            parties: None,
        };
        let t = open_deal(&w.repo, &w.clock, &w.secrets, cmd).await.unwrap();
        for (role, room) in [(Role::A, "desk-b"), (Role::B, "desk-s")] {
            let cmd = RecordDesk {
                deal: "D-0008".parse().unwrap(),
                role,
                room: RoomId::new(room).unwrap(),
            };
            record_desk(&w.repo, &w.clock, cmd).await.unwrap();
        }
        (w, [t.role_a, t.role_b])
    }

    async fn play(
        &self,
        round: u16,
        seals: [Option<&str>; 2],
        buyer_agent: &str,
    ) -> Result<String, String> {
        let plan = Plan {
            deal: "D-0008".parse().unwrap(),
            round,
            seals: seals.map(|s| s.map(Secret::new)),
            owners: [pid("u-asha"), pid("u-ravi")],
            agents: [pid(buyer_agent), pid("seller-agent")],
        };
        let ports = Ports {
            repo: &self.repo,
            vault: &self.vault,
            clock: &self.clock,
            secrets: &self.secrets,
            switch: &self.switch,
            excluded: vec![pid("notary")],
        };
        play(ports, plan).await.map_err(|e| e.0)
    }

    fn cards(&self) -> usize {
        self.switch
            .calls()
            .into_iter()
            .filter(|c| matches!(c, SwitchCall::Post { room, .. } if room.as_str() == "deal"))
            .count()
    }
}

#[tokio::test]
async fn seeds_the_deal_at_the_round_with_the_buyer_to_move() {
    let (w, [a, b]) = World::new().await;
    let summary = w
        .play(3, [Some(&a), Some(&b)], "buyer-agent-2")
        .await
        .unwrap();
    assert!(summary.contains("last O-5"), "{summary}");

    let stored = w
        .repo
        .load(&"D-0008".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let d = &stored.deal;
    assert_eq!(d.state(), DealState::Open);
    assert_eq!(d.round(), 3);
    assert_eq!(d.turn(), Role::A);
    let last = d.latest_offer().unwrap();
    assert_eq!(last.by, Role::B);
    let Terms::PurchaseOrder(t) = &last.terms else {
        panic!()
    };
    assert_eq!(t.price_per_kg().minor, SELLER_LAST);
    assert_eq!(
        d.binding(Role::A).unwrap().agent,
        Some(pid("buyer-agent-2"))
    );
    assert_eq!(w.cards(), 5);
}

#[tokio::test]
async fn rerun_resumes_and_refuses_divergence() {
    let (w, [a, b]) = World::new().await;
    // A run that stops after sealing the buyer resumes with only the seller's token.
    let e = w
        .play(3, [Some(&a), None], "buyer-agent")
        .await
        .unwrap_err();
    assert!(e.contains("seller is not sealed yet"), "{e}");
    let e = w
        .play(3, [Some(&a), Some(&b)], "buyer-agent")
        .await
        .unwrap_err();
    assert!(e.contains("buyer is already sealed"), "{e}");
    let summary = w.play(3, [None, Some(&b)], "buyer-agent").await.unwrap();
    assert!(summary.contains("(0 already there)"), "{summary}");
    assert_eq!(w.cards(), 5);
    let summary = w.play(3, [None, None], "buyer-agent").await.unwrap();
    assert!(summary.contains("(5 already there)"), "{summary}");
    assert_eq!(w.cards(), 5, "a complete fixture posts nothing");
    let e = w.play(3, [None, None], "buyer-agent-2").await.unwrap_err();
    assert!(e.contains("negotiator is already buyer-agent"), "{e}");
    // Another round's script prices the offers differently, so it stops at O-1.
    let e = w.play(4, [None, None], "buyer-agent").await.unwrap_err();
    assert!(e.contains("O-1 is not the scripted one"), "{e}");
}

#[tokio::test]
async fn a_version_conflict_is_retried() {
    let (w, [a, b]) = World::new().await;
    w.repo.1.store(2, std::sync::atomic::Ordering::SeqCst);
    let summary = w
        .play(3, [Some(&a), Some(&b)], "buyer-agent")
        .await
        .unwrap();
    assert!(summary.contains("last O-5"), "{summary}");
}
