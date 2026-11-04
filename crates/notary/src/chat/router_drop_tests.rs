use super::*;

#[tokio::test]
async fn duplicate_message_is_dropped() {
    let t = T::new(false);
    t.ready().await;
    let m = t.msg(DEAL_ROOM, ASHA, SenderKind::User, "@notary status");
    assert!(matches!(
        t.r.handle(m.clone()).await.unwrap(),
        Routed::Handled
    ));
    let before = t.calls().len();
    let again = t.r.handle(m).await.unwrap();
    assert!(matches!(again, Routed::Dropped(Dropped::Duplicate)));
    assert_eq!(t.calls().len(), before);
}

#[tokio::test]
async fn refusal_addressed_to_the_notary_is_ignored() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    let body = "@notary You're not permitted to direct messages to me in this room — my operator has restricted who can address me here.";
    let r = t.send(DESK_SELLER, SELLER_AGENT, body).await;
    assert!(matches!(r, Routed::Dropped(Dropped::Refusal)));
    let body = "@notary I only take instructions from my owner, and this chat account isn't linked";
    let r = t.send(DEAL_ROOM, BUYER_AGENT, body).await;
    assert!(matches!(r, Routed::Dropped(Dropped::Refusal)));
    assert_eq!(t.calls().len(), before);
    // A person typing the refusal text is not a refusal, and neither is a body with a
    // notary command on a later line: both are answered, never silently dropped.
    let r = t.send(DEAL_ROOM, ASHA, body).await;
    assert!(matches!(r, Routed::Handled));
    let hidden = format!("{body}\n@notary status");
    let r = t.send(DEAL_ROOM, BUYER_AGENT, &hidden).await;
    assert!(matches!(r, Routed::Handled));
    assert_eq!(t.posts_since(before).len(), 2);
}

#[tokio::test]
async fn platform_messages_other_than_operator_kickoffs_are_ignored() {
    let t = T::new(false);
    let mut m = t.msg(
        DEAL_ROOM,
        "platform-id",
        SenderKind::Platform,
        "open purchase-order deal D-0007",
    );
    m.on_behalf_of = Some("someone-else".into());
    assert!(matches!(
        t.r.handle(m).await.unwrap(),
        Routed::Dropped(Dropped::Platform)
    ));
    let m = t.msg(
        DEAL_ROOM,
        "platform-id",
        SenderKind::Platform,
        "@notary status",
    );
    assert!(matches!(
        t.r.handle(m).await.unwrap(),
        Routed::Dropped(Dropped::Platform)
    ));
    let r = t
        .send(DEAL_ROOM, ASHA, "@notary open purchase-order deal D-0007")
        .await;
    assert!(matches!(
        r,
        Routed::Dropped(Dropped::KickoffNotFromOperator)
    ));
    assert!(t.calls().is_empty());
    assert!(t.r.ports().repo.load(&deal_id()).await.unwrap().is_none());
}

#[tokio::test]
async fn kickoff_in_a_thread_is_answered_at_the_room_root() {
    let t = T::new(false);
    let r = t
        .kickoff(DEAL_ROOM, "open purchase-order deal D-0007")
        .await;
    assert!(matches!(r, Routed::Opened { .. }));
    // The only post call has no thread: every reply is a root post.
    let posts = t.posts_since(0);
    assert_eq!(
        posts,
        vec![(
            DEAL_ROOM.to_string(),
            "D-0007 open · purchase-order · waiting for seals".to_string()
        )]
    );
    let again = t
        .kickoff(DEAL_ROOM, "open purchase-order deal D-0007")
        .await;
    assert!(matches!(again, Routed::Handled));
    assert_eq!(t.posts_since(0)[1].1, "D-0007 is already open");
}

#[tokio::test]
async fn desk_side_must_fit_the_deal_kind() {
    let t = T::new(false);
    t.kickoff(DESK_BUYER, "desk tenant of D-0007").await;
    t.kickoff(DEAL_ROOM, "open purchase-order deal D-0007")
        .await;
    t.kickoff(DESK_SELLER, "desk b of D-0007").await;
    let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
    assert_eq!(stored.deal.desk(Role::A), None);
    assert_eq!(stored.deal.desk(Role::B), Some(&room(DESK_SELLER)));
    let posts = t.posts_since(0);
    assert!(posts.contains(&(
        DESK_BUYER.to_string(),
        "side does not fit a purchase-order deal; use buyer, seller, a or b".to_string()
    )));
}

#[tokio::test]
async fn probe_in_deal_room_is_refused_with_no_content() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    let body = format!("@notary probe {}", po(98_000, 400, 560, 20));
    t.send(DEAL_ROOM, BUYER_AGENT, &body).await;
    assert_eq!(
        t.posts_since(before),
        vec![(DEAL_ROOM.to_string(), "probe refused by notary".to_string())]
    );
    let before = t.calls().len();
    t.send(DESK_BUYER, BUYER_AGENT, &body).await;
    let posts = t.posts_since(before);
    assert_eq!(posts.len(), 1);
    assert!(posts[0].0 == DESK_BUYER && posts[0].1.contains("payment_net_days"));
}

#[tokio::test]
async fn offer_refused_in_deal_room_shows_no_field_names() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    let body = format!(r#"@notary offer {{"terms":{}}}"#, po(98_000, 400, 560, 20));
    t.send(DEAL_ROOM, BUYER_AGENT, &body).await;
    let posts = t.posts_since(before);
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0],
        (DEAL_ROOM.to_string(), "offer refused by notary".to_string())
    );
    // The mover's own desk still learns which of its own fields failed.
    assert_eq!(posts[1].0, DESK_BUYER);
    assert!(posts[1].1.contains("OutsideOwnLimits") && posts[1].1.contains("payment_net_days"));
    // Out of turn from the deal room: outcome line there, the reason only in the desk.
    let before = t.calls().len();
    let body = format!(r#"@notary offer {{"terms":{}}}"#, po(108_000, 400, 560, 40));
    t.send(DEAL_ROOM, SELLER_AGENT, &body).await;
    let posts = t.posts_since(before);
    assert_eq!(
        posts[0],
        (DEAL_ROOM.to_string(), "offer refused by notary".to_string())
    );
    assert!(posts[1].0 == DESK_SELLER && posts[1].1.contains("NotYourTurn"));
}

#[tokio::test]
async fn unbound_sender_and_wrong_room_are_refused() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    t.send(DEAL_ROOM, "asha-lookalike", "@notary sign 3f9a1c2e")
        .await;
    t.send(DEAL_ROOM, BUYER_AGENT, "@notary state").await;
    t.send(DEAL_ROOM, ASHA, "@notary join AAAA-BBBB").await;
    t.send(DESK_BUYER, ASHA, "@notary sign 3f9a1c2e").await;
    let posts = t.posts_since(before);
    assert_eq!(
        posts[0],
        (DEAL_ROOM.to_string(), "not a party to D-0007".to_string())
    );
    assert_eq!(posts[1], (DEAL_ROOM.to_string(), "wrong room".to_string()));
    assert_eq!(
        posts[2],
        (
            DEAL_ROOM.to_string(),
            "join only in your desk room".to_string()
        )
    );
    assert!(posts[3].0 == DESK_BUYER && posts[3].1.contains("WrongRoom"));
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn hash_mismatch_uses_the_design_text() {
    let t = T::new(false);
    t.ready().await;
    t.agree().await;
    let before = t.calls().len();
    t.send(DEAL_ROOM, ASHA, "@notary sign 00000000").await;
    let posts = t.posts_since(before);
    assert_eq!(
        posts,
        vec![(
            DEAL_ROOM.to_string(),
            "hash does not match the current terms".to_string()
        )]
    );
}

#[test]
fn inbound_maps_a_switch_message() {
    let data = r#"{"type":"message","room_id":"deal-room","payload":{"addressed":true,
        "sender":"@op:switch","sender_name":"switch","sender_kind":"platform","on_behalf_of":"operator-user",
        "message_id":"m9","body":"open lease deal D-0009","timestamp":1,"thread_id":"t1"}}"#;
    let event = proxenos_switch_client::stream::Event::parse(data).unwrap();
    let m = Inbound::from_event(&event).unwrap();
    assert_eq!(m.sender_kind, SenderKind::Platform);
    assert!(matches!(m.command(), Ok(Command::Open { .. })));
}

#[tokio::test]
async fn canned_auto_replies_posted_as_an_agent_are_ignored() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    for body in [
        "@notary I'm currently offline — my connector isn't reporting in. I'll respond once it reconnects.",
        "Hi! Mention @seller-agent to get my attention.",
        "@notary I read room messages asynchronously, not in real time — @admin has to trigger me to pull the latest messages, either from a session I already have open or by starting a new one.",
    ] {
        let r = t.send(DESK_SELLER, SELLER_AGENT, body).await;
        assert!(matches!(r, Routed::Dropped(Dropped::AutoReply)), "{body}");
        let r = t.send(DEAL_ROOM, "stranger-agent-id", body).await;
        assert!(matches!(r, Routed::Dropped(Dropped::AutoReply)), "{body}");
    }
    assert_eq!(t.calls().len(), before);
}

#[tokio::test]
async fn unparsed_text_from_an_unbound_agent_gets_no_reply() {
    let t = T::new(false);
    t.ready().await;
    let before = t.calls().len();
    for room_id in [DEAL_ROOM, DESK_BUYER] {
        let r = t
            .send(room_id, "stranger-agent-id", "@notary hello there")
            .await;
        assert!(matches!(r, Routed::Dropped(Dropped::UnboundAgentText)));
    }
    assert_eq!(t.calls().len(), before);
    // A person gets an answer to the same text.
    let r = t.send(DEAL_ROOM, ASHA, "@notary hello there").await;
    assert!(matches!(r, Routed::Handled));
    assert_eq!(t.posts_since(before).len(), 1);
}
