use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use proxenos_app::fakes::{
    CountingSecrets, FakeAdmin, FakeSigner, FakeVault, FixedClock, InMemoryDeals,
};
use proxenos_domain::party::ParticipantId;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::routes::{BODY_LIMIT, router};
use super::{AppState, HttpConfig, HttpPorts};

pub(super) const TOKEN: &str = "operator-token-0123";

#[derive(Default)]
pub(super) struct Fakes {
    pub repo: InMemoryDeals,
    pub vault: FakeVault,
    pub clock: FixedClock,
    pub secrets: CountingSecrets,
    pub admin: FakeAdmin,
    pub signer: FakeSigner,
}

impl HttpPorts for Fakes {
    type Repo = InMemoryDeals;
    type Vault = FakeVault;
    type Clock = FixedClock;
    type Secrets = CountingSecrets;
    type Admin = FakeAdmin;
    type Signer = FakeSigner;
    fn repo(&self) -> &InMemoryDeals {
        &self.repo
    }
    fn vault(&self) -> &FakeVault {
        &self.vault
    }
    fn clock(&self) -> &FixedClock {
        &self.clock
    }
    fn secrets(&self) -> &CountingSecrets {
        &self.secrets
    }
    fn admin(&self) -> &FakeAdmin {
        &self.admin
    }
    fn signer(&self) -> &FakeSigner {
        &self.signer
    }
}

pub(super) fn app_with(ports: Fakes) -> (Router, Arc<AppState<Fakes>>) {
    let state = Arc::new(AppState::new(ports, test_config()));
    (router(state.clone()), state)
}

pub(super) fn test_config() -> HttpConfig {
    HttpConfig {
        operator_token: TOKEN.into(),
        public_url: "https://notary.test".into(),
        excluded: vec![ParticipantId::new("notary-1").unwrap()],
        deal_secs: 86_400,
        max_rounds: 5,
    }
}

pub(super) fn app() -> Router {
    app_with(Fakes::default()).0
}

pub(super) async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, v)
}

pub(super) fn open_body(with_parties: bool) -> Value {
    let mut b = json!({
        "deal_id": "D-0007", "kind": "purchase-order", "room_id": "deal-room",
        "desks": {"buyer": "desk-b", "seller": "desk-s"}, "max_rounds": 12,
    });
    if with_parties {
        b["parties"] = json!({
            "buyer": {"owner_id": "u-asha", "agent_id": "a-buyer"},
            "seller": {"owner_id": "u-ravi", "agent_id": "a-seller"},
        });
    }
    b
}

/// Opens D-0007 and returns (buyer token, seller token).
pub(super) async fn open(app: &Router, with_parties: bool) -> (String, String) {
    let (status, v) = call(
        app,
        "POST",
        "/api/deals",
        Some(TOKEN),
        Some(open_body(with_parties)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let token = |side: &str| {
        let link = v["seal_links"][side].as_str().unwrap();
        assert!(link.starts_with("https://notary.test/seal/"));
        link.rsplit('/').next().unwrap().to_owned()
    };
    (token("buyer"), token("seller"))
}

pub(super) fn buyer_limits() -> Value {
    json!({"limits": {
        "price_per_kg": {"at_most": {"money": {"minor": 115_000, "currency": "INR"}}},
        "quantity_kg": {"between": [{"quantity": {"value": 300, "unit": "kg"}}, {"quantity": {"value": 500, "unit": "kg"}}]},
        "deliver_by": {"no_later_than": 40},
        "payment_net_days": {"at_least": {"integer": 30}},
    }})
}

pub(super) fn seller_limits() -> Value {
    json!({"limits": {
        "price_per_kg": {"at_least": {"money": {"minor": 102_000, "currency": "INR"}}},
        "quantity_kg": {"between": [{"quantity": {"value": 200, "unit": "kg"}}, {"quantity": {"value": 600, "unit": "kg"}}]},
        "deliver_by": {"no_earlier_than": 29},
        "payment_net_days": {"at_most": {"integer": 45}},
    }})
}

#[tokio::test]
async fn health_needs_no_auth() {
    let (status, _) = call(&app(), "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn seal_form_lists_fields_and_directions_without_values() {
    let app = app();
    let (buyer, _) = open(&app, false).await;
    let (status, v) = call(&app, "GET", &format!("/api/seal/{buyer}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["side"], "buyer");
    assert_eq!(v["kind"], "purchase-order");
    let fields = v["fields"].as_array().unwrap();
    assert_eq!(
        fields[0],
        json!({"field": "price_per_kg", "type": "money", "direction": "at_most"})
    );
    assert_eq!(fields[1]["unit"], "kg");
    assert_eq!(fields[2]["direction"], "no_later_than");
}

#[tokio::test]
async fn sealing_returns_the_join_code_once() {
    let app = app();
    let (buyer, _) = open(&app, false).await;
    let path = format!("/api/seal/{buyer}");
    let (status, v) = call(&app, "POST", &path, None, Some(buyer_limits())).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["join_code"].as_str().unwrap().len(), 9);
    assert_eq!(v["bound"], false);
    let (again, _) = call(&app, "POST", &path, None, Some(buyer_limits())).await;
    assert_eq!(again, StatusCode::NOT_FOUND);
    assert_eq!(
        call(&app, "GET", &path, None, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn bad_limits_are_refused_without_sealing() {
    let app = app();
    let (buyer, seller) = open(&app, false).await;
    // Seller-shaped limits on the buyer's token point the wrong way.
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/seal/{buyer}"),
        None,
        Some(seller_limits()),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/seal/{seller}"),
        None,
        Some(json!({"limits": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/seal/{buyer}"),
        None,
        Some(buyer_limits()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "token still usable");
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let app = app();
    let big = json!({"limits": {}, "pad": "x".repeat(BODY_LIMIT)});
    let (status, _) = call(&app, "POST", "/api/seal/abc", None, Some(big)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn timeline_shows_state_round_and_no_limits() {
    let app = app();
    let (buyer, _) = open(&app, false).await;
    call(
        &app,
        "POST",
        &format!("/api/seal/{buyer}"),
        None,
        Some(buyer_limits()),
    )
    .await;
    let (status, v) = call(&app, "GET", "/api/deals/D-0007/timeline", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["state"], "AwaitingSeals");
    assert_eq!(v["round"], 1);
    assert_eq!(v["max_rounds"], 12);
    assert_eq!(v["agreed_terms"], Value::Null);
    let text = v.to_string();
    assert!(!text.contains("115000"), "{text}");
    assert_eq!(
        call(&app, "GET", "/api/deals/D-0099/timeline", None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", "/api/deals/nope/timeline", None, None)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn receipt_is_404_until_bound_and_key_is_public() {
    let app = app();
    open(&app, false).await;
    assert_eq!(
        call(&app, "GET", "/api/deals/D-0007/receipt", None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, v) = call(&app, "GET", "/api/notary/key", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        v["public_key"],
        proxenos_app::secret::hex(FakeSigner::PUBLIC_KEY)
    );
}
