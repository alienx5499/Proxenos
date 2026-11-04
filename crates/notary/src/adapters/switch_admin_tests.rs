use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::adapters::mock::Mock;

fn admin(mock: &Mock) -> GatewayAdmin {
    GatewayAdmin::new(&mock.base, "op-key", "notary-1", "br1", false).unwrap()
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

const OWNER_ONLY: &str = r#"{"id":"notary-1","addressing_policy":{"rules":[{"rooms":"*","room_groups":"*","users":[],"agents":[],"owner":true,"owner_agents":false,"platform":false}]}}"#;

/// Serves the owner-only policy until a PUT lands, then the policy with the party rule
/// (or, with `lose_writes`, keeps serving owner-only as if another writer overwrote it).
async fn policy_mock(lose_writes: bool) -> Mock {
    let written = Arc::new(AtomicBool::new(false));
    let after = json!({"addressing_policy": {"rules": [
        {"owner": true, "users": [], "agents": []},
        party_rule("g1", &ids(&["u1", "u2"]), &ids(&["a1", "a2"])),
    ]}})
    .to_string();
    Mock::start(move |method, _| {
        if method == "PUT" {
            written.store(!lose_writes, Ordering::SeqCst);
            return (200, "{}".to_owned());
        }
        let body = if written.load(Ordering::SeqCst) {
            after.clone()
        } else {
            OWNER_ONLY.to_owned()
        };
        (200, body)
    })
    .await
}

#[tokio::test]
async fn admit_appends_one_rule_keeps_the_owner_rule_and_verifies() {
    let mock = policy_mock(false).await;
    let a = admin(&mock);
    a.admit_parties("g1", &ids(&["u1", "u2"]), &ids(&["a1", "a2"]))
        .await
        .unwrap();
    let puts = mock.seen_path("/gateway/agents/notary-1/addressing-policy");
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].method, "PUT");
    assert_eq!(puts[0].header("authorization").unwrap(), "Bearer op-key");
    let rules = puts[0].json()["policy"]["rules"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["owner"], true);
    assert_eq!(
        rules[1],
        party_rule("g1", &ids(&["u1", "u2"]), &ids(&["a1", "a2"]))
    );
    assert_eq!(
        mock.seen_path("/gateway/agents/notary-1").len(),
        2,
        "read, then verify"
    );
}

#[tokio::test]
async fn a_lost_write_is_retried_once_then_fails() {
    let mock = policy_mock(true).await;
    let e = admin(&mock)
        .admit_parties("g1", &ids(&["u1", "u2"]), &ids(&["a1", "a2"]))
        .await
        .unwrap_err();
    assert!(e.0.contains("missing"), "{e}");
    assert_eq!(
        mock.seen_path("/gateway/agents/notary-1/addressing-policy")
            .len(),
        2
    );
}

#[tokio::test]
async fn concurrent_admits_are_serialized() {
    let mock = policy_mock(false).await;
    let a = admin(&mock);
    let (u, ag) = (ids(&["u1", "u2"]), ids(&["a1", "a2"]));
    let (x, y) = tokio::join!(
        a.admit_parties("g1", &u, &ag),
        a.admit_parties("g1", &u, &ag)
    );
    x.unwrap();
    y.unwrap();
    assert_eq!(
        mock.seen_path("/gateway/agents/notary-1/addressing-policy")
            .len(),
        1
    );
}

#[test]
fn gateway_url_must_be_https_off_loopback() {
    assert!(GatewayAdmin::new("http://switch.example", "k", "n", "b", false).is_err());
    assert!(GatewayAdmin::new("https://switch.example", "k", "n", "b", false).is_ok());
    assert!(GatewayAdmin::new("http://localhost:8000", "k", "n", "b", false).is_ok());
    assert!(GatewayAdmin::new("http://[::1]:8000", "k", "n", "b", false).is_ok());
    let docker = "http://host.docker.internal:8000";
    assert!(GatewayAdmin::new(docker, "k", "n", "b", false).is_err());
    assert!(GatewayAdmin::new(docker, "k", "n", "b", true).is_ok());
}

#[test]
fn ambiguous_user_is_an_error() {
    let users = json!([
        {"id": "ext-1", "external_username": "asha"},
        {"id": "ext-2", "external_username": "asha"},
    ]);
    assert!(pick_user(&users, "asha").unwrap_err().0.contains("several"));
}

#[tokio::test]
async fn admit_is_idempotent_on_rule_content() {
    let policy = json!({"addressing_policy": {"rules": [
        {"owner": true, "users": [], "agents": []},
        {"room_groups": ["g1"], "users": ["u2", "u1"], "agents": ["a1", "a2"]},
    ]}})
    .to_string();
    let mock = Mock::start(move |_, _| (200, policy.clone())).await;
    admin(&mock)
        .admit_parties("g1", &ids(&["u1", "u2"]), &ids(&["a1", "a2"]))
        .await
        .unwrap();
    assert!(
        mock.seen_path("/gateway/agents/notary-1/addressing-policy")
            .is_empty()
    );
}

#[test]
fn open_policy_starts_from_the_owner_rule() {
    for open in [Value::Null, json!({"rules": []})] {
        let rules = with_rule(&open, party_rule("g", &[], &[])).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["owner"], true);
    }
}

#[tokio::test]
async fn lookups_read_groups_and_bridge_users() {
    let mock = Mock::start(|_, path| {
        let body = match path {
            "/gateway/room-groups" => {
                r#"[{"id":"g7","name":"D-0007 cafe-roaster"},{"id":"g8","name":"D-0008 x"}]"#
            }
            "/gateway/collaborations/br1/users" => {
                r#"[{"id":"ext-1","bridge_id":"br1","external_user_id":"mm-1","external_username":"asha"}]"#
            }
            _ => "{}",
        };
        (200, body.to_owned())
    })
    .await;
    let a = admin(&mock);
    assert_eq!(a.group_id("D-0007").await.unwrap(), "g7");
    assert_eq!(a.group_id("D-0008 x").await.unwrap(), "g8");
    assert!(a.group_id("D-0009").await.is_err());
    assert_eq!(a.external_user_id("asha").await.unwrap(), "ext-1");
    assert_eq!(a.external_user_id("ext-1").await.unwrap(), "ext-1");
    assert!(a.external_user_id("kiran").await.is_err());
}

#[tokio::test]
async fn gateway_errors_surface() {
    let mock = Mock::start(|_, _| (403, r#"{"detail":"no"}"#.to_owned())).await;
    let e = admin(&mock).group_id("D-0007").await.unwrap_err();
    assert!(e.0.contains("403"), "{e}");
}
