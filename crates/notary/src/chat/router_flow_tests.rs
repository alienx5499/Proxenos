use super::*;

fn has(posts: &[(String, String)], room_id: &str, text: &str) -> bool {
    posts.iter().any(|(r, t)| r == room_id && t.contains(text))
}

#[tokio::test]
async fn purchase_order_runs_from_kickoff_to_bound_by_chat_alone() {
    let t = T::new(false);
    t.ready().await;
    let posts = t.posts_since(0);
    assert!(has(
        &posts,
        DEAL_ROOM,
        "D-0007 open · purchase-order · waiting for seals"
    ));
    assert!(has(&posts, DESK_BUYER, "desk for buyer of D-0007"));
    assert!(has(&posts, DESK_SELLER, "desk for seller of D-0007"));
    assert!(has(&posts, DESK_BUYER, "bound: buyer of D-0007"));
    assert!(has(&posts, DESK_BUYER, "@buyer-agent send state to bind"));
    assert!(has(&posts, DEAL_ROOM, "buyer joined"));
    assert!(has(&posts, DEAL_ROOM, "seller joined"));
    assert!(has(&posts, DESK_SELLER, "negotiator bound"));
    assert!(has(&posts, DESK_BUYER, "@buyer-agent your turn"));
    assert!(!has(&posts, DESK_SELLER, "@seller-agent your turn"));
    let sheet = t.calls().iter().any(|c| {
        matches!(c, SwitchCall::WriteDocument { room: r, name, .. }
            if r.as_str() == DEAL_ROOM && name == "D-0007 deal sheet")
    });
    assert!(sheet, "deal sheet written at open");

    let before = t.calls().len();
    t.agree().await;
    let posts = t.posts_since(before);
    assert!(has(&posts, DEAL_ROOM, "**Offer O-1** · buyer"));
    assert!(has(&posts, DESK_BUYER, r#""offer_id": "O-1""#));
    assert!(has(&posts, DESK_SELLER, "@seller-agent your turn"));
    assert!(has(&posts, DEAL_ROOM, "offer refused by notary"));
    assert!(has(&posts, DESK_SELLER, "payment_net_days"));
    assert!(has(&posts, DEAL_ROOM, "**Offer O-2** · seller"));
    assert!(has(&posts, DEAL_ROOM, "**Terms agreed · D-0007**"));
    let hash8 = t.hash8().await;
    assert!(has(&posts, DESK_BUYER, &format!(r#""agreed": "{hash8}""#)));

    t.send(DEAL_ROOM, ASHA, &format!("@notary sign {hash8}"))
        .await;
    t.send(DEAL_ROOM, RAVI, &format!("@notary sign {hash8}"))
        .await;
    let posts = t.posts_since(before);
    assert!(has(&posts, DEAL_ROOM, &format!("buyer signed {hash8}")));
    assert!(has(&posts, DEAL_ROOM, &format!("seller signed {hash8}")));
    assert!(has(
        &posts,
        DEAL_ROOM,
        &format!("D-0007 bound · terms {hash8}")
    ));
    let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
    assert!(matches!(
        stored.deal.state(),
        proxenos_domain::deal::DealState::Bound(_)
    ));
    assert!(stored.receipt.is_some());
    let updated = t.calls().iter().any(|c| {
        matches!(c, SwitchCall::UpdateDocument { content, .. } if content.contains("receipt signature"))
    });
    assert!(updated, "deal sheet rewritten at Bound");

    for text in t.deal_room_posts() {
        assert!(
            !text.contains("payment_net_days") && !text.contains("failing"),
            "{text}"
        );
    }
    t.assert_no_stray_mentions();
}

#[tokio::test]
async fn sign_cards_bind_through_polled_outcomes() {
    let t = T::new(true);
    t.ready().await;
    t.agree().await;
    let requests: Vec<_> = t
        .calls()
        .iter()
        .filter_map(|c| match c {
            SwitchCall::RequestSignature(card) => Some(card.role),
            _ => None,
        })
        .collect();
    assert_eq!(requests, vec![Role::A, Role::B]);
    let stored = t.r.ports().repo.load(&deal_id()).await.unwrap().unwrap();
    for req in &stored.sign_requests {
        let owner = if req.role == Role::A { ASHA } else { RAVI };
        let answer = proxenos_app::ports::CardAnswer::Sign(ParticipantId::new(owner).unwrap());
        t.r.ports()
            .switch
            .push_outcome(proxenos_app::ports::CardOutcome {
                request: req.request_id.clone(),
                answer,
            });
    }
    assert_eq!(t.r.process_card_outcomes().await.unwrap(), 2);
    assert_eq!(
        t.r.process_card_outcomes().await.unwrap(),
        0,
        "acked outcomes are gone"
    );
    let hash8 = t.hash8().await;
    assert!(t.deal_room_posts().iter().any(|p| p
        == &format!(
            "D-0007 bound · terms {hash8} · receipt: http://notary.test/api/deals/D-0007/receipt"
        )));
    t.assert_no_stray_mentions();
}
