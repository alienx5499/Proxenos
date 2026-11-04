//! Calendar dates at the router: past dates are bad commands; state shows today.

use super::*;
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;

const TODAY: i32 = 550;

fn iso_offer(day: i32) -> String {
    let terms = po(108_000, 400, 0, 40).replace(
        r#""deliver_by":0"#,
        &format!(r#""deliver_by":"{}""#, Date::from_days(day).iso()),
    );
    format!(r#"@notary offer {{"terms":{terms}}}"#)
}

async fn ready_on_today() -> T {
    let t = T::new(false);
    let secs = i64::from(TODAY) * 86_400 + 3_600;
    t.r.ports().clock.set(Timestamp(secs));
    t.ready().await;
    t
}

async fn offers(t: &T) -> usize {
    let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
    stored.deal.offers().len()
}

#[tokio::test]
async fn past_dates_are_refused_as_bad_commands() {
    let t = ready_on_today().await;
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &iso_offer(TODAY - 1)).await;
    let legacy = format!(r#"@notary offer {{"terms":{}}}"#, po(108_000, 400, 329, 40));
    t.send(DESK_BUYER, BUYER_AGENT, &legacy).await;
    let probe = format!("@notary probe {}", po(108_000, 400, TODAY - 1, 40));
    t.send(DESK_BUYER, BUYER_AGENT, &probe).await;
    t.send(DEAL_ROOM, BUYER_AGENT, &legacy).await;
    let posts = t.posts_since(before);
    let desk: Vec<_> = posts.iter().filter(|(r, _)| r == DESK_BUYER).collect();
    assert_eq!(desk.len(), 3, "{posts:?}");
    for (_, text) in desk {
        assert!(text.contains(r#""error": "BadCommand""#), "{text}");
        assert!(text.contains("date is in the past"), "{text}");
    }
    assert!(posts.contains(&(DEAL_ROOM.into(), "date is in the past".into())));
    assert_eq!(offers(&t).await, 0, "nothing reached the deal");
}

#[tokio::test]
async fn iso_offer_on_or_after_today_is_recorded() {
    let t = ready_on_today().await;
    t.send(DESK_BUYER, BUYER_AGENT, &iso_offer(TODAY + 10))
        .await;
    assert_eq!(offers(&t).await, 1);
    let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
    let Terms::PurchaseOrder(p) = &stored.deal.offers()[0].terms else {
        panic!("purchase order expected");
    };
    assert_eq!(p.deliver_by(), Date::from_days(TODAY + 10));
}

#[tokio::test]
async fn state_block_carries_today_and_iso_offer_dates() {
    let t = ready_on_today().await;
    t.send(DESK_BUYER, BUYER_AGENT, &iso_offer(TODAY + 10))
        .await;
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, "@notary state").await;
    let posts = t.posts_since(before);
    let (_, text) = posts.iter().find(|(r, _)| r == DESK_BUYER).unwrap();
    let today = Date::from_days(TODAY).iso();
    assert!(text.contains(&format!(r#""today": "{today}""#)), "{text}");
    let due = Date::from_days(TODAY + 10).iso();
    assert!(
        text.contains(&format!(r#""deliver_by": "{due}""#)),
        "{text}"
    );
    let wakes = t.posts_since(0);
    let wake = wakes
        .iter()
        .find(|(_, x)| x.starts_with("@seller-agent your turn"));
    assert!(wake.unwrap().1.contains(&format!(r#""today": "{today}""#)));
}
