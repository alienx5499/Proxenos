//! Boots the serve wiring against a mock Switch: kickoffs over the stream, seals over HTTP,
//! then the deal-room post and the opener's wake must reach Switch.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use super::{build, run};
use crate::adapters::mock::Mock;
use crate::config::Config;

const OPERATOR: &str = "op-1";

fn kickoff(seq: i64, room: &str, body: &str) -> String {
    let payload = json!({
        "addressed": true, "sender": "@switch-platform:x", "sender_name": "Switch",
        "sender_kind": "platform", "on_behalf_of": OPERATOR, "message_id": format!("k{seq}"),
        "body": body, "timestamp": 1_000, "thread_id": "t1", "attachments": [],
    });
    let event = json!({"type": "message", "room_id": room, "payload": payload, "sequence": seq});
    format!("id: {seq}\nevent: message\ndata: {event}\n\n")
}

fn routes(_: &str, path: &str) -> (u16, String) {
    let agents = |names: &[&str]| {
        let list: Vec<Value> = names
            .iter()
            .map(|n| json!({"type": "agent", "agent_id": n, "name": n, "status": null}))
            .collect();
        json!({ "participants": list }).to_string()
    };
    let body = match path {
        "/agents/ag1/message" => json!({"event_id": "e1"}).to_string(),
        "/agents/ag1/ops/create_room_document" => {
            json!({"result": {"document_id": "d1"}}).to_string()
        }
        "/agents/rooms/desk-b/participants" => agents(&["notary", "buyer-agent"]),
        "/agents/rooms/desk-s/participants" => agents(&["notary", "seller-agent"]),
        _ => json!({"ok": true, "rooms": [], "cursor": 0}).to_string(),
    };
    (200, body)
}

fn config(mock: &Mock, db: &std::path::Path) -> Config {
    let hex = "ab".repeat(32);
    let env: HashMap<&str, String> = HashMap::from([
        ("PROXENOS_VAULT_KEY", hex.clone()),
        ("PROXENOS_SECRET_KEY", "cd".repeat(32)),
        ("PROXENOS_RECEIPT_KEY", "ef".repeat(32)),
        ("SWITCH_URL", mock.base.clone()),
        ("SWITCH_AGENT_ID", "ag1".into()),
        ("SWITCH_AGENT_KEY", "agent-key".into()),
        ("SWITCH_GATEWAY_URL", mock.base.clone()),
        ("SWITCH_GATEWAY_KEY", "gateway-key".into()),
        ("SWITCH_BRIDGE_ID", "br1".into()),
        ("PROXENOS_OPERATOR_ID", OPERATOR.into()),
        ("PROXENOS_OPERATOR_TOKEN", "operator-token-0123".into()),
        ("PROXENOS_NOTARY_SENDER_ID", "@notary:x".into()),
        ("PROXENOS_DB", db.display().to_string()),
        ("PROXENOS_PUBLIC_URL", "https://notary.test".into()),
    ]);
    Config::from_lookup(|k| env.get(k).cloned()).unwrap()
}

fn limits(buyer: bool) -> Value {
    let money = |v: i64| json!({"money": {"minor": v, "currency": "INR"}});
    let kg = |v: u32| json!({"quantity": {"value": v, "unit": "kg"}});
    if buyer {
        json!({"limits": {"price_per_kg": {"at_most": money(115_000)},
            "quantity_kg": {"between": [kg(300), kg(500)]},
            "deliver_by": {"no_later_than": 40}, "payment_net_days": {"at_least": {"integer": 30}}}})
    } else {
        json!({"limits": {"price_per_kg": {"at_least": money(102_000)},
            "quantity_kg": {"between": [kg(200), kg(600)]},
            "deliver_by": {"no_earlier_than": 29}, "payment_net_days": {"at_most": {"integer": 45}}}})
    }
}

/// Polls the mock until a `/message` post to `room` satisfies `want`.
async fn posted(mock: &Mock, room: &str, want: impl Fn(&str) -> bool) -> String {
    for _ in 0..200 {
        let found = mock
            .seen_path("/agents/ag1/message")
            .iter()
            .map(|s| s.json())
            .find(|v| v["room_id"] == room && want(v["content"].as_str().unwrap_or_default()));
        if let Some(v) = found {
            return v["content"].as_str().unwrap().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no matching post in {room}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kickoff_seal_and_first_wake_flow_through_the_wiring() {
    let frames = [
        kickoff(
            1,
            "deal-room",
            "open purchase-order deal D-0007 cafe-roaster",
        ),
        kickoff(2, "desk-b", "desk buyer of D-0007"),
        kickoff(3, "desk-s", "desk seller of D-0007"),
    ]
    .concat();
    let mock = Mock::streaming(frames, routes).await;
    let dir = std::env::temp_dir().join(format!("proxenos-serve-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = config(&mock, &dir.join("notary.db"));

    let wired = build(&cfg).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let (links_tx, mut links) = mpsc::unbounded_channel();
    let server = tokio::spawn(run(
        wired,
        listener,
        async { drop(stop_rx.await) },
        links_tx,
    ));

    let opened = posted(&mock, "deal-room", |t| t.starts_with("D-0007 open")).await;
    assert!(opened.contains("purchase-order"), "{opened}");
    let mut tokens = HashMap::new();
    for _ in 0..2 {
        let line = tokio::time::timeout(Duration::from_secs(10), links.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(line.starts_with("SEAL LINK D-0007 "), "{line}");
        let side = line
            .split_whitespace()
            .nth(3)
            .unwrap()
            .trim_end_matches(':')
            .to_owned();
        let token = line.rsplit('/').next().unwrap().to_owned();
        assert!(line.contains("https://notary.test/seal/"));
        tokens.insert(side, token);
    }
    let seals = mock.seen_path("/agents/ag1/message");
    assert!(
        seals.iter().all(|s| !s.json()["content"]
            .as_str()
            .unwrap()
            .contains(&tokens["buyer"])),
        "seal tokens are never posted to Switch"
    );

    let http = reqwest::Client::new();
    for (side, buyer) in [("buyer", true), ("seller", false)] {
        // The desk kickoffs may still be in the queue; sealing needs no desk, the wake does.
        let resp = http
            .post(format!("{base}/api/seal/{}", tokens[side]))
            .json(&limits(buyer))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "{side}: {}",
            resp.text().await.unwrap()
        );
    }
    let wake = posted(&mock, "desk-b", |t| t.starts_with("@buyer-agent your turn")).await;
    assert!(wake.contains("\"my_role\""), "{wake}");

    stop_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Ok(()));

    // A restart with another vault key fails at once instead of on every later move.
    let mut wrong = cfg.clone();
    wrong.vault_key = crate::config::Secret::new("12".repeat(32));
    let e = build(&wrong).await.err().unwrap();
    assert!(e.contains("PROXENOS_VAULT_KEY does not open"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_lost_receipt_key_is_refused_once_deals_are_bound() {
    let dir = std::env::temp_dir().join(format!("proxenos-key-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("notary.db");
    let store = crate::store::sqlite::Store::open(&db).await.unwrap();
    sqlx::query(
        "INSERT INTO deals (id, kind, room_id, state, terminal, round, max_rounds, deadline, \
         version, body_json) VALUES ('D-0001', 'purchase_order', 'r', 'bound', 1, 1, 8, 0, 1, '{}')",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let mock = Mock::start(routes).await;
    let mut cfg = config(&mock, &db);
    cfg.receipt_key = None;
    cfg.receipt_key_file = dir.join("receipt.key");
    let e = build(&cfg).await.err().unwrap();
    assert!(e.contains("is missing but 1 bound deals"), "{e}");
    assert!(!cfg.receipt_key_file.exists(), "no new key is minted");
    std::fs::remove_dir_all(&dir).unwrap();
}
