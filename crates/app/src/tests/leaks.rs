//! No-leak sweep: serialize everything a scripted run produces and look for sealed values.

use proxenos_domain::kind::DealKind;
use proxenos_domain::party::Role;
use serde::Serialize;
use serde_json::Value;

use super::fixtures::*;
use super::support::*;
use crate::AppError;
use crate::commands::accept_offer::AcceptOutcome;
use crate::commands::bind_negotiator::{BindNegotiator, bind_negotiator};
use crate::commands::record_signature::SignOutcome;
use crate::commands::withdraw::{Withdraw, withdraw};
use crate::ports::CardAnswer;
use crate::ports::DealReader;
use crate::queries::get_party_view::get_party_view;
use crate::queries::get_receipt::get_receipt;
use crate::queries::get_status::get_status;
use crate::queries::get_timeline::get_timeline;
use crate::queries::list_deals::list_deals;

const D: &str = "D-0007";

#[derive(Default)]
struct Sweep(Vec<Value>);

impl Sweep {
    fn add(&mut self, v: &impl Serialize) {
        self.0.push(serde_json::to_value(v).unwrap());
    }

    async fn snapshot(&mut self, h: &H) {
        for role in [Role::A, Role::B] {
            self.add(&get_party_view(&h.repo, &id(D), role).await.unwrap());
            self.add(&get_status(&h.repo, &id(D), Some(role)).await.unwrap());
        }
        self.add(&get_timeline(&h.repo, &id(D)).await.unwrap());
        self.add(&list_deals(&h.repo, &deal_room()).await.unwrap());
    }
}

/// Every number (or all-digit string) anywhere in `v`.
fn numbers(v: &Value, out: &mut Vec<i64>) {
    match v {
        Value::Number(n) => out.extend(n.as_i64()),
        Value::String(s) => out.extend(s.parse::<i64>().ok()),
        Value::Array(items) => items.iter().for_each(|i| numbers(i, out)),
        Value::Object(map) => map.values().for_each(|i| numbers(i, out)),
        _ => {}
    }
}

impl Sweep {
    fn result<T: Serialize>(&mut self, r: Result<T, AppError>) -> Option<T> {
        match r {
            Ok(v) => {
                self.add(&v);
                Some(v)
            }
            Err(e) => {
                self.add(&e.to_string());
                self.add(&format!("{e:?}"));
                None
            }
        }
    }
}

#[tokio::test]
async fn no_sealed_value_appears_in_any_output() {
    let h = H::default();
    let mut sweep = Sweep::default();
    let tokens = h.open(D, DealKind::PurchaseOrder).await;
    sweep.add(&tokens);
    let po_limits = |r| limits(DealKind::PurchaseOrder, r);
    sweep.result(h.seal(&tokens.role_a, po_limits(Role::B)).await);
    let mut codes = Vec::new();
    for (role, token) in [(Role::A, &tokens.role_a), (Role::B, &tokens.role_b)] {
        let sealed = sweep.result(h.seal(token, po_limits(role)).await).unwrap();
        sweep.result(h.join("AAAA-AAAA", pid("guesser"), desk(role)).await);
        sweep.result(h.join(&sealed.join_code, owner(role), desk(role)).await);
        codes.push(sealed.join_code);
        let cmd = BindNegotiator {
            deal: id(D),
            sender: agent(role),
            room: desk(role),
            excluded: vec![],
        };
        sweep.result(bind_negotiator(&h.repo, &h.clock, cmd).await);
    }
    sweep.result(h.join(&codes[0], agent(Role::A), desk(Role::A)).await);
    sweep.snapshot(&h).await;

    let probes = [
        (Role::A, po(98_000, 400, 560, 35)),
        (Role::A, po(120_000, 600, 700, 20)),
        (Role::B, po(90_000, 100, 400, 60)),
    ];
    for (role, terms) in probes {
        sweep.result(h.probe(D, role, desk(role), terms).await);
        sweep.result(
            h.probe(D, role, deal_room(), po(98_000, 400, 560, 35))
                .await,
        );
    }
    sweep.result(h.offer(D, Role::A, po(98_000, 400, 560, 35)).await);
    sweep.result(h.offer(D, Role::B, po(112_000, 400, 560, 60)).await);
    sweep.result(
        h.offer_in(D, Role::B, po(112_000, 400, 560, 60), deal_room())
            .await,
    );
    sweep.result(h.offer(D, Role::A, po(98_000, 400, 560, 35)).await);
    sweep.result(h.offer(D, Role::B, po(112_000, 400, 560, 40)).await);
    sweep.snapshot(&h).await;
    sweep.result(h.offer(D, Role::A, po(108_000, 400, 560, 40)).await);
    sweep.result(h.accept_in(D, Role::A, "O-3", deal_room()).await);
    let agreed = sweep.result(h.accept(D, Role::B, "O-3").await).unwrap();
    let AcceptOutcome::Agreed { hash } = agreed else {
        panic!("expected agreement")
    };
    sweep.snapshot(&h).await;
    sweep.result(h.sign(D, agent(Role::A), &hash.short()).await);
    sweep.result(h.sign(D, owner(Role::A), "00000000").await);
    let card = h.open_card(D, Role::A).await.unwrap();
    sweep.result(
        h.card(D, card.clone(), CardAnswer::Sign(owner(Role::B)))
            .await,
    );
    sweep.result(h.card(D, card, CardAnswer::Sign(owner(Role::A))).await);
    let card = h.open_card(D, Role::A).await.unwrap();
    sweep.result(h.card(D, card, CardAnswer::Decline(owner(Role::A))).await);
    sweep.result(h.sign(D, owner(Role::A), &hash.short()).await);
    let bound = sweep.result(h.sign(D, owner(Role::B), &hash.short()).await);
    assert!(matches!(bound, Some(SignOutcome::Bound { .. })));
    sweep.result(get_receipt(&h.repo, &id(D)).await);
    sweep.result(
        withdraw(
            &h.repo,
            &h.clock,
            Withdraw {
                deal: id(D),
                sender: owner(Role::A),
            },
        )
        .await,
    );
    sweep.snapshot(&h).await;
    sweep.add(&h.repo.events(&id(D)).await.unwrap());
    sweep.add(&h.repo.load(&id(D)).await.unwrap().unwrap());

    let mut seen = Vec::new();
    sweep.0.iter().for_each(|v| numbers(v, &mut seen));
    assert!(seen.contains(&108_000), "sweep must include offer terms");
    let leaked: Vec<_> = PO_SEALED.iter().filter(|s| seen.contains(s)).collect();
    assert!(leaked.is_empty(), "sealed values leaked: {leaked:?}");
    let text = serde_json::to_string(&sweep.0).unwrap();
    for sealed in PO_SEALED {
        let n = sealed.to_string();
        let embedded = text.match_indices(&n).any(|(i, _)| {
            let before = text[..i]
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric());
            let after = text[i + n.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric());
            !before && !after
        });
        assert!(
            !embedded,
            "sealed value {n} appears as a token in output text"
        );
    }
}
