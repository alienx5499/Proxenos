use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};

use proxenos_app::commands::seal_limits::{SealLimits, seal_limits};
use proxenos_app::fakes::{FakeRoster, SwitchCall};
use proxenos_app::queries::get_status::get_status;
use proxenos_domain::date::Date;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::limit::{Constraint, LimitSet};
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::party::ParticipantId;
use proxenos_domain::quantity::Quantity;
use proxenos_domain::terms::{FieldValue, PurchaseOrderTerms};

use super::*;

#[path = "router_fakes_tests.rs"]
mod fakes;
use fakes::Fakes;

#[path = "router_date_tests.rs"]
mod dates;
#[path = "router_drop_tests.rs"]
mod drops;
#[path = "router_flow_tests.rs"]
mod flow;
#[path = "router_review_tests.rs"]
mod review;
#[path = "router_wake_tests.rs"]
mod wakes;

const OPERATOR: &str = "operator-user";
const DEAL_ROOM: &str = "deal-room";
const DESK_BUYER: &str = "desk-buyer";
const DESK_SELLER: &str = "desk-seller";
const ASHA: &str = "asha-id";
const RAVI: &str = "ravi-id";
const BUYER_AGENT: &str = "buyer-agent-id";
const SELLER_AGENT: &str = "seller-agent-id";

fn room(s: &str) -> RoomId {
    RoomId::new(s).unwrap()
}

fn deal_id() -> DealId {
    "D-0007".parse().unwrap()
}

fn po(price: i64, kg: u32, day: i32, net: u16) -> String {
    let t = PurchaseOrderTerms::new(
        Money::new(price, Currency::Inr),
        Quantity::kg(kg),
        Date::from_days(day),
        net,
    )
    .unwrap();
    serde_json::to_string(&proxenos_domain::terms::Terms::PurchaseOrder(t)).unwrap()
}

fn limits(role: Role) -> LimitSet {
    use Constraint::*;
    let inr = |m| FieldValue::Money(Money::new(m, Currency::Inr));
    let kg = |v| FieldValue::Quantity(Quantity::kg(v));
    let map = match role {
        Role::A => [
            (Field::PricePerKg, AtMost(inr(115_037))),
            (Field::QuantityKg, Between(kg(311), kg(493))),
            (Field::DeliverBy, NoLaterThan(Date::from_days(617))),
            (Field::PaymentNetDays, AtLeast(FieldValue::Integer(31))),
        ],
        Role::B => [
            (Field::PricePerKg, AtLeast(inr(102_019))),
            (Field::QuantityKg, Between(kg(207), kg(607))),
            (Field::DeliverBy, NoEarlierThan(Date::from_days(503))),
            (Field::PaymentNetDays, AtMost(FieldValue::Integer(47))),
        ],
    };
    LimitSet::new(DealKind::PurchaseOrder, role, BTreeMap::from(map)).unwrap()
}

struct T {
    r: Router<Fakes>,
    n: AtomicU32,
}

impl T {
    fn new(sign_cards: bool) -> Self {
        Self::with_buyer_desk(sign_cards, &["notary", "buyer-agent"])
    }

    /// `buyer_desk` is the agent roster of the buyer's desk room.
    fn with_buyer_desk(sign_cards: bool, buyer_desk: &[&str]) -> Self {
        let roster = FakeRoster(HashMap::from([
            (
                room(DESK_BUYER),
                buyer_desk.iter().map(|n| n.to_string()).collect(),
            ),
            (
                room(DESK_SELLER),
                vec!["notary".into(), "seller-agent".into()],
            ),
        ]));
        let fakes = Fakes {
            roster,
            ..Fakes::default()
        };
        let mut cfg = RouterConfig::new(ParticipantId::new("notary-id").unwrap(), OPERATOR);
        cfg.sign_cards = sign_cards;
        T {
            r: Router::new(fakes, cfg),
            n: AtomicU32::new(0),
        }
    }

    fn msg(&self, room_id: &str, sender: &str, kind: SenderKind, body: &str) -> Inbound {
        let n = self.n.fetch_add(1, Ordering::SeqCst) + 1;
        Inbound {
            message_id: format!("m{n}"),
            room: room(room_id),
            sender: ParticipantId::new(sender).unwrap(),
            sender_name: sender.trim_end_matches("-id").into(),
            sender_kind: kind,
            on_behalf_of: (kind == SenderKind::Platform).then(|| OPERATOR.into()),
            body: body.into(),
            addressed: true,
        }
    }

    async fn send(&self, room_id: &str, sender: &str, body: &str) -> Routed {
        let kind = if sender.ends_with("agent-id") {
            SenderKind::Agent
        } else {
            SenderKind::User
        };
        self.r
            .handle(self.msg(room_id, sender, kind, body))
            .await
            .unwrap()
    }

    /// A template kickoff: platform voice for the operator.
    async fn kickoff(&self, room_id: &str, body: &str) -> Routed {
        let m = self.msg(room_id, "platform-id", SenderKind::Platform, body);
        self.r.handle(m).await.unwrap()
    }

    fn calls(&self) -> Vec<SwitchCall> {
        self.r.ports().switch.calls()
    }

    /// Posts since call index `from`, as (room, text).
    fn posts_since(&self, from: usize) -> Vec<(String, String)> {
        self.calls()[from..]
            .iter()
            .filter_map(|c| match c {
                SwitchCall::Post { room, text } => Some((room.to_string(), text.clone())),
                _ => None,
            })
            .collect()
    }

    async fn hash8(&self) -> String {
        let s = get_status(&self.r.ports().repo, &deal_id(), None)
            .await
            .unwrap();
        s.hash8.unwrap()
    }

    /// Kickoffs, seals, joins and negotiator binding: the deal is Open, buyer to move.
    async fn ready(&self) {
        let codes = self.sealed().await;
        self.send(DESK_BUYER, ASHA, &format!("@notary join {}", codes[0]))
            .await;
        self.send(DESK_SELLER, RAVI, &format!("@notary join {}", codes[1]))
            .await;
        self.send(DESK_BUYER, BUYER_AGENT, "@notary state").await;
        self.send(DESK_SELLER, SELLER_AGENT, "@notary state").await;
    }

    /// Kickoffs and both seals; returns the two join codes, buyer first.
    async fn sealed(&self) -> Vec<String> {
        assert!(matches!(
            self.kickoff(DESK_BUYER, "desk buyer of D-0007").await,
            Routed::Handled
        ));
        let opened = self
            .kickoff(DEAL_ROOM, "open purchase-order deal D-0007 cafe-roaster")
            .await;
        let Routed::Opened { seal_tokens, .. } = opened else {
            panic!("expected the deal to open");
        };
        self.kickoff(DESK_SELLER, "@notary desk seller of D-0007")
            .await;
        let p = self.r.ports();
        let mut codes = Vec::new();
        for (role, token) in [(Role::A, seal_tokens.role_a), (Role::B, seal_tokens.role_b)] {
            let cmd = SealLimits {
                token,
                limits: limits(role),
            };
            let sealed = seal_limits(&p.repo, &p.vault, &p.clock, &p.secrets, cmd).await;
            codes.push(sealed.unwrap().join_code);
        }
        codes
    }

    /// Buyer offers, seller is refused once, seller counters, buyer accepts.
    async fn agree(&self) {
        self.send(
            DESK_BUYER,
            BUYER_AGENT,
            &format!(r#"@notary offer {{"terms":{}}}"#, po(98_000, 400, 560, 35)),
        )
        .await;
        self.send(
            DESK_SELLER,
            SELLER_AGENT,
            &format!(r#"@notary offer {{"terms":{}}}"#, po(112_000, 400, 560, 60)),
        )
        .await;
        self.send(
            DESK_SELLER,
            SELLER_AGENT,
            &format!(
                r#"@notary offer {{"terms":{},"note":"ping @buyer-agent"}}"#,
                po(108_000, 400, 560, 40)
            ),
        )
        .await;
        self.send(DESK_BUYER, BUYER_AGENT, "@notary accept O-2")
            .await;
    }

    /// `@` only at the start of notary replies addressed to a desk's bound negotiator, plus
    /// turn wakes: each in that negotiator's own desk, with exactly one `@`.
    fn assert_no_stray_mentions(&self) {
        for (room_id, text) in self.posts_since(0) {
            if text.starts_with("@buyer-agent ") {
                assert_eq!(room_id, DESK_BUYER);
            } else if text.starts_with("@seller-agent ") {
                assert_eq!(room_id, DESK_SELLER);
            } else {
                assert!(!text.contains('@'), "stray mention in {room_id}: {text}");
                continue;
            }
            assert_eq!(text.matches('@').count(), 1, "{text}");
        }
    }

    fn deal_room_posts(&self) -> Vec<String> {
        let posts = self.posts_since(0).into_iter();
        posts
            .filter(|(r, _)| r == DEAL_ROOM)
            .map(|(_, t)| t)
            .collect()
    }
}
