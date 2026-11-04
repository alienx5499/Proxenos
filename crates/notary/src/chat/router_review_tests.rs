use std::sync::atomic::Ordering;

use super::*;

async fn stored(t: &T, deal: &str) -> Option<StoredDeal> {
    let id: DealId = deal.parse().unwrap();
    t.r.ports().repo.load(&id).await.unwrap()
}

#[tokio::test]
async fn switch_down_after_open_keeps_seal_tokens_and_the_sheet_is_retried() {
    let t = T::new(false);
    t.r.ports().switch.down.store(true, Ordering::SeqCst);
    let opened = t
        .kickoff(DEAL_ROOM, "open purchase-order deal D-0007")
        .await;
    assert!(matches!(opened, Routed::Opened { .. }));
    assert!(stored(&t, "D-0007").await.unwrap().sheet_dirty);
    assert!(
        t.r.retry_dirty_sheets().await.is_err(),
        "still down: error returned"
    );

    t.r.ports().switch.down.store(false, Ordering::SeqCst);
    assert_eq!(t.r.process_card_outcomes().await.unwrap(), 0);
    let deal = stored(&t, "D-0007").await.unwrap();
    assert!(!deal.sheet_dirty);
    assert_eq!(deal.deal_sheet_doc.as_deref(), Some("doc-1"));
}

#[tokio::test]
async fn kickoff_names_the_opener() {
    let t = T::new(false);
    let opened = t
        .kickoff(
            DEAL_ROOM,
            "open purchase-order deal D-0008 opener seller roaster",
        )
        .await;
    assert!(matches!(opened, Routed::Opened { .. }));
    assert_eq!(stored(&t, "D-0008").await.unwrap().deal.opener(), Role::B);

    let t = T::new(false);
    let r = t
        .kickoff(DEAL_ROOM, "open purchase-order deal D-0009 opener tenant")
        .await;
    assert!(matches!(r, Routed::Handled));
    assert!(
        stored(&t, "D-0009").await.is_none(),
        "label must fit the kind"
    );
}

#[tokio::test]
async fn an_agent_cannot_type_join_and_burns_no_attempt() {
    let t = T::new(false);
    let codes = t.sealed().await;
    let before = t.calls().len();
    t.send(
        DESK_BUYER,
        BUYER_AGENT,
        &format!("@notary join {}", codes[0]),
    )
    .await;
    let posts = t.posts_since(before);
    assert_eq!(posts.len(), 1);
    assert!(posts[0].0 == DESK_BUYER && posts[0].1.contains("AgentCannotJoin"));
    let deal = stored(&t, "D-0007").await.unwrap();
    assert_eq!(deal.party(Role::A).failed_joins, 0);
    assert!(deal.party(Role::A).join_code.is_some());
    assert!(deal.deal.binding(Role::A).is_none());
}

#[tokio::test]
async fn only_state_or_kinds_binds_a_negotiator_and_never_the_operator() {
    let t = T::new(false);
    let codes = t.sealed().await;
    t.send(DESK_BUYER, ASHA, &format!("@notary join {}", codes[0]))
        .await;
    let before = t.calls().len();
    let offer = format!(r#"@notary offer {{"terms":{}}}"#, po(98_000, 400, 560, 35));
    t.send(DESK_BUYER, BUYER_AGENT, &offer).await;
    t.send(DESK_BUYER, OPERATOR, "@notary state").await;
    let posts = t.posts_since(before);
    assert!(posts[0].1.contains("NotAParty"), "{posts:?}");
    assert!(posts[1].1.contains("ExcludedNegotiator"), "{posts:?}");
    let binding = stored(&t, "D-0007")
        .await
        .unwrap()
        .deal
        .binding(Role::A)
        .cloned();
    assert_eq!(binding.unwrap().agent, None);

    t.send(DESK_BUYER, BUYER_AGENT, "@notary kinds").await;
    let deal = stored(&t, "D-0007").await.unwrap();
    let agent = deal.deal.binding(Role::A).unwrap().agent.clone();
    assert_eq!(agent.unwrap().as_str(), BUYER_AGENT);
}

#[tokio::test]
async fn turn_wakes_mention_only_the_bound_negotiator() {
    let t = T::with_buyer_desk(false, &["notary", "spy-agent", "buyer-agent"]);
    t.ready().await;
    let deal = stored(&t, "D-0007").await.unwrap();
    let name = deal.party(Role::A).negotiator_name.clone();
    assert_eq!(name.as_deref(), Some("buyer-agent"));
    let turn_wakes: Vec<String> = t
        .posts_since(0)
        .into_iter()
        .filter(|(r, text)| r == DESK_BUYER && text.contains("your turn"))
        .map(|(_, text)| text)
        .collect();
    assert_eq!(turn_wakes.len(), 1);
    assert!(turn_wakes[0].starts_with("@buyer-agent your turn\n"));
    assert!(!turn_wakes[0].contains("spy-agent"));
}

#[tokio::test]
async fn a_version_conflict_is_retried_and_the_move_lands() {
    let t = T::new(false);
    t.ready().await;
    t.r.ports().repo.1.store(2, Ordering::SeqCst);
    let offer = format!(r#"@notary offer {{"terms":{}}}"#, po(98_000, 400, 560, 35));
    t.send(DESK_BUYER, BUYER_AGENT, &offer).await;
    assert_eq!(stored(&t, "D-0007").await.unwrap().deal.offers().len(), 1);
}

#[tokio::test]
async fn a_lasting_conflict_is_reported_in_the_room_and_returned() {
    let t = T::new(false);
    t.ready().await;
    t.r.ports()
        .repo
        .1
        .store(CONFLICT_ATTEMPTS, Ordering::SeqCst);
    let offer = format!(r#"@notary offer {{"terms":{}}}"#, po(98_000, 400, 560, 35));
    let msg = t.msg(DESK_BUYER, BUYER_AGENT, SenderKind::Agent, &offer);
    let from = t.calls().len();
    assert!(t.r.handle(msg).await.is_err());
    assert_eq!(
        t.posts_since(from),
        [(DESK_BUYER.to_string(), replies::not_recorded())]
    );
}

#[tokio::test]
async fn a_flooding_sender_is_dropped() {
    let t = T::new(false);
    t.sealed().await;
    let mut routed = Vec::new();
    for _ in 0..21 {
        routed.push(t.send(DEAL_ROOM, ASHA, "@notary status").await);
    }
    assert!(matches!(routed[19], Routed::Handled));
    assert!(matches!(routed[20], Routed::Dropped(Dropped::RateLimited)));
}
