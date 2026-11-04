use proxenos_app::ports::{
    CardAnswer, RequestId, RosterPort, SignCard, SignatureOutcomes, SwitchPort,
};
use proxenos_domain::hash::TermsHash;
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use proxenos_switch_client::approvals::ApprovalView;
use proxenos_switch_client::{Credentials, SwitchClient};
use serde_json::json;

use super::mock::Mock;
use super::switch::{SwitchAdapter, outcome, sign_card};

fn adapter(mock: &Mock) -> SwitchAdapter {
    let creds = Credentials {
        agent_id: "ag1".into(),
        api_key: "key1".into(),
    };
    SwitchAdapter::new(SwitchClient::new(&mock.base, creds).unwrap(), None)
}

fn room() -> RoomId {
    RoomId::new("r1").unwrap()
}

fn hash() -> TermsHash {
    "3f9a1c2e".repeat(8).parse().unwrap()
}

fn card(role: Role, attempt: usize) -> SignCard {
    SignCard {
        room: room(),
        deal: "D-0007".parse().unwrap(),
        role,
        label: if role == Role::A { "buyer" } else { "seller" }.into(),
        terms_hash: hash(),
        round: 5,
        expires: Timestamp(86_399),
        attempt,
    }
}

fn ok_routes(_: &str, path: &str) -> (u16, String) {
    let body = match path {
        p if p.ends_with("/ops/create_room_document") => r#"{"result":{"document_id":"d9"}}"#,
        p if p.ends_with("/participants") => {
            r#"{"participants":[{"type":"agent","agent_id":"a1","name":"buyer-agent","status":null},
                {"type":"user","agent_id":null,"name":"asha","status":null}]}"#
        }
        p if p.ends_with("/approvals") => {
            r#"{"sessionId":"s","requestId":"sign-x","kind":"approval","state":"open"}"#
        }
        _ => r#"{"ok":true,"rooms":[],"cursor":0}"#,
    };
    (200, body.to_owned())
}

#[tokio::test]
async fn post_uses_message_route() {
    let mock = Mock::start(ok_routes).await;
    let a = adapter(&mock);
    a.post(&room(), "hello").await.unwrap();
    let posts = mock.seen_path("/agents/ag1/message");
    let bodies: Vec<_> = posts.iter().map(|s| s.json()["content"].clone()).collect();
    assert_eq!(bodies, [json!("hello")]);
    assert_eq!(posts[0].json()["room_id"], "r1");
    assert_eq!(posts[0].header("authorization").unwrap(), "Bearer key1");
}

#[tokio::test]
async fn documents_run_on_a_one_shot_room_connection_and_release_it() {
    let mock = Mock::start(ok_routes).await;
    let a = adapter(&mock);
    let doc = a
        .write_document(&room(), "D-0007 deal sheet", "body")
        .await
        .unwrap();
    assert_eq!(doc.0, "d9");
    a.update_document(&room(), &doc, "body2").await.unwrap();
    let create = &mock.seen_path("/agents/ag1/ops/create_room_document")[0];
    assert_eq!(create.json()["name"], "D-0007 deal sheet");
    assert_eq!(create.json()["content"], "body");
    assert!(create.header("x-switch-connection-id").is_some());
    let update = &mock.seen_path("/agents/ag1/ops/update_room_document")[0];
    assert_eq!(
        update.json(),
        json!({"document_id": "d9", "content": "body2"})
    );
    assert_eq!(
        mock.seen_path("/agents/ag1/connection/unsubscribe").len(),
        2
    );
}

#[tokio::test]
async fn roster_keeps_agent_names_only() {
    let mock = Mock::start(ok_routes).await;
    let names = adapter(&mock).agent_names(&room()).await.unwrap();
    assert_eq!(names, ["buyer-agent"]);
}

#[tokio::test]
async fn sign_card_sends_every_field() {
    let mock = Mock::start(ok_routes).await;
    let a = adapter(&mock);
    let id = a.request_signature(&card(Role::A, 1)).await.unwrap();
    assert_eq!(id, RequestId("sign-x".into()));
    let seen = &mock.seen.lock().unwrap().clone();
    let open = seen
        .iter()
        .find(|s| s.path.ends_with("/approvals"))
        .unwrap();
    let body = open.json();
    for key in [
        "request_id",
        "turn_id",
        "title",
        "detail",
        "options",
        "room_id",
        "thread_id",
        "expires_at",
        "kind",
        "questions",
    ] {
        assert!(body.get(key).is_some(), "missing {key}");
    }
    assert_eq!(body["thread_id"], json!(null));
    assert_eq!(body["questions"], json!([]));
    assert_eq!(body["options"][0]["id"], "sign");
    assert_eq!(body["options"][1]["decision"], "decline");
    assert_eq!(body["request_id"], "sign-D-0007-buyer-3f9a1c2e");
    assert_eq!(body["turn_id"], "D-0007-r5");
    let title = body["title"].as_str().unwrap();
    assert!(
        title.contains("buyer") && title.contains("3f9a1c2e"),
        "{title}"
    );
}

#[test]
fn card_ids_are_unique_per_attempt() {
    let a = sign_card(&card(Role::B, 1));
    let b = sign_card(&card(Role::B, 2));
    assert_eq!(a.request_id, "sign-D-0007-seller-3f9a1c2e");
    assert_eq!(b.request_id, "sign-D-0007-seller-3f9a1c2e-2");
    assert_eq!(a.turn_id, b.turn_id);
}

fn view(state: &str, answer: Option<&str>, by: Option<&str>) -> ApprovalView {
    serde_json::from_value(json!({
        "sessionId": "s", "requestId": "r", "kind": "approval", "state": state,
        "answer": answer, "answeredBy": by,
    }))
    .unwrap()
}

#[test]
fn outcomes_map_to_card_answers() {
    let asha = ParticipantId::new("u-asha").unwrap();
    let cases = [
        (
            view("answered", Some("sign"), Some("u-asha")),
            Some(CardAnswer::Sign(asha.clone())),
        ),
        (
            view("answered", Some("decline"), Some("u-asha")),
            Some(CardAnswer::Decline(asha)),
        ),
        (
            view("answered", Some("sign"), None),
            Some(CardAnswer::Expired),
        ),
        (view("expired", None, None), Some(CardAnswer::Expired)),
        (view("open", None, None), None),
    ];
    for (v, want) in cases {
        assert_eq!(outcome(&v).map(|o| o.answer), want, "{v:?}");
    }
}

#[tokio::test]
async fn ack_goes_to_the_outcome_session() {
    let mock = Mock::start(|_, path| {
        let body = if path.ends_with("/outcomes") {
            r#"[{"sessionId":"old","requestId":"sign-1","kind":"approval","state":"expired"}]"#
        } else {
            r#"{"sessionId":"old","requestId":"sign-1","kind":"approval","state":"expired"}"#
        };
        (200, body.to_owned())
    })
    .await;
    let a = adapter(&mock);
    let got = a.undelivered().await.unwrap();
    assert_eq!(got.len(), 1);
    a.ack_outcome(&got[0].request).await.unwrap();
    assert_eq!(
        mock.seen_path("/agent-sessions/old/approvals/sign-1/delivered")
            .len(),
        1
    );
}
