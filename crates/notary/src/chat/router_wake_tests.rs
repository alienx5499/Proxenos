use super::*;

fn offer(price: i64) -> String {
    format!(r#"@notary offer {{"terms":{}}}"#, po(price, 400, 560, 35))
}

#[tokio::test]
async fn refused_offer_in_desk_wakes_the_negotiator_with_error_and_state() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &offer(120_000)).await;
    let desk: Vec<String> = t
        .posts_since(before)
        .into_iter()
        .filter(|(r, _)| r == DESK_BUYER)
        .map(|(_, t)| t)
        .collect();
    let [wake] = desk.as_slice() else {
        panic!("one desk reply expected: {desk:?}");
    };
    assert!(wake.starts_with("@buyer-agent your command was refused\n```json\n"));
    assert!(wake.contains("OutsideOwnLimits") && wake.contains("price_per_kg"));
    assert!(wake.contains(r#""my_role": "buyer""#) && wake.contains(r#""today""#));

    let before = t.calls().len();
    t.send(DESK_SELLER, SELLER_AGENT, &offer(110_000)).await;
    let posts = t.posts_since(before);
    assert!(
        posts.iter().any(|(r, t)| r == DESK_SELLER
            && t.starts_with("@seller-agent ")
            && t.contains("NotYourTurn")),
        "{posts:?}"
    );
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn owner_bad_command_in_desk_gets_a_plain_reply() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    t.send(DESK_BUYER, ASHA, "@notary offer {broken").await;
    let posts = t.posts_since(before);
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert!(posts[0].1.starts_with("```json") && posts[0].1.contains("BadCommand"));
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn error_wakes_stop_after_the_cap_in_a_round() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    for _ in 0..8 {
        t.send(DESK_BUYER, BUYER_AGENT, "@notary offer {broken")
            .await;
    }
    let replies: Vec<String> = t.posts_since(before).into_iter().map(|(_, t)| t).collect();
    assert_eq!(replies.len(), 8);
    assert!(
        replies[..6]
            .iter()
            .all(|r| r.starts_with("@buyer-agent your command was refused\n"))
    );
    assert!(replies[6].starts_with("@buyer-agent ") && replies[6].contains("last refusal wake"));
    assert!(
        replies[7].starts_with("```json"),
        "quiet after the hold wake"
    );

    t.send(DESK_BUYER, BUYER_AGENT, &offer(98_000)).await;
    let before = t.calls().len();
    t.send(DESK_SELLER, SELLER_AGENT, "@notary offer {broken")
        .await;
    let posts = t.posts_since(before);
    assert!(posts[0].1.starts_with("@seller-agent "), "{posts:?}");
    t.assert_no_stray_mentions();
}

fn desk_posts(t: &T, room_id: &str, before: usize) -> Vec<String> {
    let posts = t.posts_since(before).into_iter();
    posts
        .filter(|(r, _)| r == room_id)
        .map(|(_, x)| x)
        .collect()
}

#[tokio::test]
async fn negotiator_replies_in_its_desk_are_addressed() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    let probe = format!("@notary probe {}", po(98_000, 400, 560, 35));
    t.send(DESK_BUYER, BUYER_AGENT, &probe).await;
    let [p] = desk_posts(&t, DESK_BUYER, before).try_into().unwrap();
    assert!(p.starts_with("@buyer-agent probe result\n```json\n"), "{p}");
    assert!(p.contains(r#""probes_left""#) && p.contains(r#""my_role": "buyer""#));
    assert!(
        p.contains(r#""today""#),
        "its turn, so the state block follows"
    );

    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, "@notary state").await;
    let [s] = desk_posts(&t, DESK_BUYER, before).try_into().unwrap();
    assert!(s.starts_with("@buyer-agent state\n```json\n") && s.contains(r#""today""#));

    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &offer(98_000)).await;
    let [ack] = desk_posts(&t, DESK_BUYER, before).try_into().unwrap();
    assert!(
        ack.starts_with("@buyer-agent offer recorded\n```json\n"),
        "{ack}"
    );
    assert!(ack.contains(r#""offer_id": "O-1""#));

    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &probe).await;
    let [off_turn] = desk_posts(&t, DESK_BUYER, before).try_into().unwrap();
    assert!(off_turn.starts_with("@buyer-agent probe result\n"));
    assert!(
        !off_turn.contains(r#""today""#),
        "not its turn: no state block"
    );
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn owner_status_in_desk_is_not_addressed() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    t.send(DESK_BUYER, ASHA, "@notary status").await;
    let [s] = desk_posts(&t, DESK_BUYER, before).try_into().unwrap();
    assert!(
        s.starts_with("D-0007 · ") && s.contains("probes left"),
        "{s}"
    );
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn repeating_own_last_terms_is_refused_without_a_move() {
    let t = T::new(false);
    t.ready().await;
    t.send(DESK_BUYER, BUYER_AGENT, &offer(98_000)).await;
    let counter = format!(r#"@notary offer {{"terms":{}}}"#, po(108_000, 400, 560, 40));
    t.send(DESK_SELLER, SELLER_AGENT, &counter).await;
    for room in [DESK_BUYER, DEAL_ROOM] {
        let before = t.calls().len();
        t.send(room, BUYER_AGENT, &offer(98_000)).await;
        let posts = t.posts_since(before);
        assert!(
            posts.iter().any(|(r, t)| r == DESK_BUYER
                && t.contains("BadCommand")
                && t.contains("same terms as your last offer")),
            "{posts:?}"
        );
        let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
        assert_eq!(stored.deal.offers().len(), 2);
    }
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &offer(99_000)).await;
    let posts = t.posts_since(before);
    assert!(
        posts
            .iter()
            .any(|(_, t)| t.contains(r#""offer_id": "O-3""#))
    );
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn a_valid_offer_after_the_hold_wake_is_recorded_and_acknowledged() {
    let t = T::new(false);
    t.ready().await;
    for _ in 0..7 {
        t.send(DESK_BUYER, BUYER_AGENT, &offer(120_000)).await;
    }
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &offer(98_000)).await;
    let desk: Vec<String> = t
        .posts_since(before)
        .into_iter()
        .filter(|(r, _)| r == DESK_BUYER)
        .map(|(_, t)| t)
        .collect();
    assert!(
        desk.iter()
            .any(|m| m.starts_with("@buyer-agent ") && m.contains("offer recorded")),
        "{desk:?}"
    );
}
