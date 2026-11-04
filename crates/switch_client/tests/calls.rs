mod common;

use std::collections::BTreeMap;

use common::{Mock, Script, Sse, state_frame};
use proxenos_switch_client::approvals::{ApprovalCard, ApprovalOption, Decision};
use proxenos_switch_client::ops::Upload;
use proxenos_switch_client::register::{Registration, register};
use serde_json::json;

fn replying(body: &str) -> Script {
    Script {
        reply: Some((200, body.to_owned())),
        ..Script::default()
    }
}

#[tokio::test]
async fn register_sends_always_on_profile_with_registration_token() {
    let mock = Mock::start(replying(r#"{"id":"ag9","api_key":"k9"}"#)).await;
    let reg = Registration {
        name: "notary".into(),
        description: "deal notary".into(),
        display_name: None,
        connector_type: "proxenos".into(),
        overwrite: false,
    };
    let creds = register(&format!("{}/", mock.base), "regtok", &reg)
        .await
        .unwrap();
    assert_eq!(
        (creds.agent_id.as_str(), creds.api_key.as_str()),
        ("ag9", "k9")
    );
    let req = &mock.seen_path("/agents")[0];
    assert_eq!(req.method, "POST");
    assert_eq!(
        req.header("authorization").as_deref(),
        Some("Bearer regtok")
    );
    let b = req.json();
    assert_eq!(b["name"], "notary");
    assert_eq!(b["connector_type"], "proxenos");
    assert_eq!(b["integration_profile"]["connection_model"], "always_on");
    assert_eq!(b["integration_profile"]["message_exchange"], true);
    assert_eq!(b["integration_profile"]["event_reporting"], json!([]));
}

#[tokio::test]
async fn post_message_and_participants() {
    let mock = Mock::start(replying(
        r#"{"ok":true,"event_id":"e1","participants":[{"type":"agent","agent_id":"a2","name":"buyer-agent","status":"live"}]}"#,
    ))
    .await;
    let client = mock.client();
    assert_eq!(
        client
            .post_message("r1", "@buyer-agent your turn")
            .await
            .unwrap()
            .as_deref(),
        Some("e1")
    );
    let msg = &mock.seen_path("/agents/ag1/message")[0];
    assert_eq!(msg.header("authorization").as_deref(), Some("Bearer key1"));
    assert_eq!(
        msg.json(),
        json!({"room_id": "r1", "content": "@buyer-agent your turn"})
    );

    let people = client.participants("r1").await.unwrap();
    assert_eq!(people[0].name, "buyer-agent");
    assert_eq!(people[0].kind, "agent");
    assert_eq!(
        mock.seen_path("/agents/rooms/r1/participants")[0].method,
        "GET"
    );
}

#[tokio::test]
async fn ops_aliases_and_media() {
    let mock = Mock::start(replying(r#"{"result":{"ok":true}}"#)).await;
    let client = mock.client();
    let aliases = BTreeMap::from([("notary".to_owned(), "clerk".to_owned())]);
    client.update_room_aliases("r1", &aliases).await.unwrap();
    let op = &mock.seen_path("/agents/ag1/ops/update_room")[0];
    assert_eq!(
        op.json(),
        json!({"room_id": "r1", "aliases": {"notary": "clerk"}})
    );
    assert_eq!(op.header("x-switch-connection-id"), None);

    let files = vec![
        Upload {
            filename: "receipt.json".into(),
            mime: "application/json".into(),
            bytes: b"{}".to_vec(),
        },
        Upload {
            filename: "po.md".into(),
            mime: "text/markdown".into(),
            bytes: b"# PO".to_vec(),
        },
    ];
    client
        .upload_media("r1", files, Some("Bound"))
        .await
        .unwrap();
    let up = &mock.seen_path("/agents/ag1/rooms/r1/media")[0];
    assert!(
        up.header("content-type")
            .unwrap()
            .starts_with("multipart/form-data")
    );
    let body = String::from_utf8_lossy(&up.body);
    assert_eq!(body.matches("name=\"files\"; filename=").count(), 2);
    assert!(body.contains("name=\"caption\"") && body.contains("Bound"));
}

#[tokio::test]
async fn room_session_binds_ops_to_its_connection_and_releases_on_close() {
    let script = Script {
        sse: [Sse {
            body: state_frame("c", 30.0),
            hold_open: true,
        }]
        .into(),
        reply: Some((200, r#"{"result":{"document_id":"d1"}}"#.into())),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let room = mock.client().open_room("deal1").await.unwrap();
    let doc = room
        .create_room_document("deal-sheet", "D-1", "read only", "body")
        .await
        .unwrap();
    assert_eq!(doc, "d1");
    room.update_room_document("d1", "body2").await.unwrap();
    room.send_targeted_message("Terms ready", &["everyone"])
        .await
        .unwrap();

    let open = &mock.seen_path("/agents/ag1/events")[0];
    let conn = open.param("connection_id").unwrap();
    assert_eq!(open.param("scope").as_deref(), Some("single"));
    assert_eq!(open.param("rooms").as_deref(), Some("deal1"));
    assert_eq!(open.param("filter").as_deref(), Some("addressed"));
    for (name, args) in [
        (
            "create_room_document",
            json!({"name":"deal-sheet","description":"D-1","instructions":"read only","content":"body"}),
        ),
        (
            "update_room_document",
            json!({"document_id":"d1","content":"body2"}),
        ),
        (
            "send_targeted_message",
            json!({"body":"Terms ready","target_names":["everyone"]}),
        ),
    ] {
        let op = &mock.seen_path(&format!("/agents/ag1/ops/{name}"))[0];
        assert_eq!(
            op.header("x-switch-connection-id").as_deref(),
            Some(conn.as_str())
        );
        assert_eq!(op.json(), args);
    }

    room.close().await.unwrap();
    let unsub = &mock.seen_path("/agents/ag1/connection/unsubscribe")[0];
    assert_eq!(
        unsub.json(),
        json!({"connection_id": conn, "room_id": "deal1"})
    );
}

#[tokio::test]
async fn approvals_send_every_field_and_read_camel_case() {
    let view = r#"{"sessionId":"s1","requestId":"sign-D-1-buyer-3f9a1c2e","kind":"approval","state":"answered",
        "answer":"sign","answers":null,"answeredBy":"@switch-user-a:x","answeredAt":"t","expiresAt":null,"deliveredAt":null}"#;
    let mock = Mock::start(replying(view)).await;
    let client = mock.client();
    let card = ApprovalCard {
        request_id: "sign-D-1-buyer-3f9a1c2e".into(),
        turn_id: "D-1-r5".into(),
        title: "Sign PO D-1".into(),
        detail: None,
        options: vec![
            ApprovalOption {
                id: "sign".into(),
                label: "Sign".into(),
                decision: Decision::Accept,
            },
            ApprovalOption {
                id: "decline".into(),
                label: "Decline".into(),
                decision: Decision::Decline,
            },
        ],
        room_id: Some("deal1".into()),
        thread_id: None,
        expires_at: Some("EXPIRES_AT".into()),
    };
    let v = client.open_approval("s1", &card).await.unwrap();
    assert_eq!(v.answered_by.as_deref(), Some("@switch-user-a:x"));
    let body = mock.seen_path("/agent-sessions/s1/approvals")[0].json();
    assert_eq!(
        body,
        json!({
            "request_id": "sign-D-1-buyer-3f9a1c2e", "turn_id": "D-1-r5", "kind": "approval",
            "title": "Sign PO D-1", "detail": null,
            "options": [
                {"id": "sign", "label": "Sign", "decision": "accept"},
                {"id": "decline", "label": "Decline", "decision": "decline"}
            ],
            "questions": [], "room_id": "deal1", "thread_id": null,
            "expires_at": "EXPIRES_AT"
        })
    );

    mock.script.lock().unwrap().reply = Some((200, format!("[{view}]")));
    let outcomes = client.approval_outcomes().await.unwrap();
    assert_eq!(outcomes[0].state, "answered");
    assert_eq!(
        mock.seen_path("/agent-sessions/approvals/outcomes")[0].method,
        "GET"
    );

    mock.script.lock().unwrap().reply = Some((200, view.to_owned()));
    client
        .ack_approval("s1", "sign-D-1-buyer-3f9a1c2e")
        .await
        .unwrap();
    let ack = &mock.seen_path("/agent-sessions/s1/approvals/sign-D-1-buyer-3f9a1c2e/delivered")[0];
    assert_eq!(ack.method, "POST");
}

#[tokio::test]
async fn dropping_a_room_session_releases_the_room() {
    let script = Script {
        sse: [Sse {
            body: state_frame("c", 30.0),
            hold_open: true,
        }]
        .into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    drop(mock.client().open_room("deal1").await.unwrap());
    for _ in 0..50 {
        if !mock
            .seen_path("/agents/ag1/connection/unsubscribe")
            .is_empty()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("room not released on drop");
}
