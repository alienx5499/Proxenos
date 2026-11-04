use std::collections::HashMap;

use axum::http::StatusCode;
use proxenos_app::fakes::FakeAdmin;
use proxenos_app::ports::DealReader;
use proxenos_domain::party::{ParticipantId, Role};
use serde_json::json;

use super::AppState;
use super::routes::router;
use super::tests::{
    Fakes, TOKEN, app, app_with, buyer_limits, call, open, open_body, seller_limits, test_config,
};

#[tokio::test]
async fn open_needs_the_operator_token() {
    let app = app();
    let body = Some(open_body(false));
    assert_eq!(
        call(&app, "POST", "/api/deals", None, body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _) = call(
        &app,
        "POST",
        "/api/deals",
        Some("operator-token-0124"),
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(&app, "POST", "/api/deals", Some(TOKEN), body.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/api/deals", Some(TOKEN), body).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn open_rejects_bad_sides_and_shared_ids() {
    let app = app();
    let mut body = open_body(false);
    body["desks"] = json!({"buyer": "desk-b"});
    assert_eq!(
        call(&app, "POST", "/api/deals", Some(TOKEN), Some(body))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut body = open_body(true);
    body["parties"]["seller"]["owner_id"] = json!("u-asha");
    assert_eq!(
        call(&app, "POST", "/api/deals", Some(TOKEN), Some(body))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut body = open_body(false);
    body["desks"] = json!({"buyer": "deal-room", "seller": "desk-s"});
    assert_eq!(
        call(&app, "POST", "/api/deals", Some(TOKEN), Some(body))
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn open_is_atomic_and_refuses_rooms_in_use() {
    let (app, state) = app_with(Fakes::default());
    // A desk equal to the deal room is refused and nothing is saved.
    let mut body = open_body(false);
    body["desks"] = json!({"buyer": "deal-room", "seller": "desk-s"});
    let (status, _) = call(&app, "POST", "/api/deals", Some(TOKEN), Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let d7 = "D-0007".parse().unwrap();
    assert!(state.ports.repo.load(&d7).await.unwrap().is_none());
    open(&app, false).await;
    let mut body = open_body(false);
    body["deal_id"] = json!("D-0008");
    body["desks"] = json!({"buyer": "desk-x", "seller": "desk-s"});
    let (status, v) = call(&app, "POST", "/api/deals", Some(TOKEN), Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("D-0007"), "{v}");
    let d8 = "D-0008".parse().unwrap();
    assert!(state.ports.repo.load(&d8).await.unwrap().is_none());
    let stored = state.ports.repo.load(&d7).await.unwrap().unwrap();
    assert_eq!(stored.deal.desk(Role::B).unwrap().as_str(), "desk-s");
}

/// The bench path: open with party ids, seal both sides, admit, read the timeline.
#[tokio::test]
async fn bench_flow_binds_both_parties_on_seal_and_admits_them() {
    let ports = Fakes {
        admin: FakeAdmin {
            groups: HashMap::from([("D-0007".into(), "g7".into())]),
            users: HashMap::from([
                ("u-asha".into(), "ext-a".into()),
                ("u-ravi".into(), "ext-r".into()),
            ]),
            ..FakeAdmin::default()
        },
        ..Fakes::default()
    };
    let (app, state) = app_with(ports);
    let (buyer, seller) = open(&app, true).await;
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/seal/{buyer}"),
        None,
        Some(buyer_limits()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["bound"], true);
    let (_, v) = call(
        &app,
        "POST",
        &format!("/api/seal/{seller}"),
        None,
        Some(seller_limits()),
    )
    .await;
    assert_eq!(v["bound"], true);

    let stored = state
        .ports
        .repo
        .load(&"D-0007".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let b = stored.deal.binding(Role::A).unwrap();
    assert_eq!(b.owner, ParticipantId::new("u-asha").unwrap());
    assert_eq!(b.agent, Some(ParticipantId::new("a-buyer").unwrap()));
    assert_eq!(b.desk.as_str(), "desk-b");
    assert!(stored.deal.binding(Role::B).unwrap().agent.is_some());

    let path = "/api/deals/D-0007/admit";
    let (status, _) = call(&app, "POST", path, Some(TOKEN), None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bound sender ids are never used"
    );
    let body = json!({"owners": ["u-asha", "u-ravi"], "agents": ["a-buyer", "a-seller"]});
    let (status, v) = call(&app, "POST", path, Some(TOKEN), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["group_id"], "g7");
    let admitted = state.ports.admin.admitted.lock().unwrap().clone();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].users, ["ext-a", "ext-r"]);
    assert_eq!(admitted[0].agents, ["a-buyer", "a-seller"]);

    let (_, v) = call(&app, "GET", "/api/deals/D-0007/timeline", None, None).await;
    assert_eq!(v["state"], "Open");
    assert_eq!(v["turn"], "buyer");
}

#[tokio::test]
async fn admit_needs_owners_and_agents_in_the_body() {
    let ports = Fakes {
        admin: FakeAdmin {
            groups: HashMap::from([("D-0007".into(), "g7".into())]),
            users: HashMap::from([
                ("asha".into(), "ext-a".into()),
                ("ravi".into(), "ext-r".into()),
            ]),
            ..FakeAdmin::default()
        },
        ..Fakes::default()
    };
    let (app, state) = app_with(ports);
    open(&app, false).await;
    let path = "/api/deals/D-0007/admit";
    assert_eq!(
        call(&app, "POST", path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "POST", path, Some(TOKEN), None).await.0,
        StatusCode::BAD_REQUEST
    );
    let body = json!({"owners": ["asha", "ravi"], "agents": ["a1", "a2"]});
    let (status, v) = call(&app, "POST", path, Some(TOKEN), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        state.ports.admin.admitted.lock().unwrap()[0].users,
        ["ext-a", "ext-r"]
    );
    assert_eq!(
        call(&app, "POST", "/api/deals/D-0042/admit", Some(TOKEN), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn switch_failures_do_not_leak_details() {
    let (app, _) = app_with(Fakes::default());
    open(&app, false).await;
    let body = json!({"owners": ["asha", "ravi"], "agents": ["a1", "a2"]});
    let (status, v) = call(
        &app,
        "POST",
        "/api/deals/D-0007/admit",
        Some(TOKEN),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(v["error"], "switch call failed");
}

#[tokio::test]
async fn the_second_seal_announces_the_open_deal() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let state = AppState::new(Fakes::default(), test_config()).notify_opened(tx);
    let app = router(std::sync::Arc::new(state));
    let (buyer, seller) = open(&app, false).await;
    call(
        &app,
        "POST",
        &format!("/api/seal/{buyer}"),
        None,
        Some(buyer_limits()),
    )
    .await;
    assert!(rx.try_recv().is_err(), "one seal does not open the deal");
    call(
        &app,
        "POST",
        &format!("/api/seal/{seller}"),
        None,
        Some(seller_limits()),
    )
    .await;
    assert_eq!(rx.try_recv().unwrap().as_str(), "D-0007");
}

#[tokio::test]
async fn open_without_max_rounds_uses_the_configured_cap() {
    let app = app();
    let mut body = open_body(false);
    body.as_object_mut().unwrap().remove("max_rounds");
    let (status, v) = call(&app, "POST", "/api/deals", Some(TOKEN), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let (_, v) = call(&app, "GET", "/api/deals/D-0007/timeline", None, None).await;
    assert_eq!(v["max_rounds"], 5);
}
