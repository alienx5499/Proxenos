use std::collections::HashMap;

use super::*;

fn full() -> HashMap<&'static str, String> {
    let hex = "ab".repeat(32);
    HashMap::from([
        ("PROXENOS_VAULT_KEY", hex.clone()),
        ("PROXENOS_SECRET_KEY", hex.clone()),
        ("SWITCH_URL", "http://switch".into()),
        ("SWITCH_AGENT_ID", "notary-1".into()),
        ("SWITCH_AGENT_KEY", "agent-secret".into()),
        ("SWITCH_GATEWAY_URL", "http://switch".into()),
        ("SWITCH_GATEWAY_KEY", "gateway-secret".into()),
        ("SWITCH_BRIDGE_ID", "br1".into()),
        ("PROXENOS_OPERATOR_ID", "op".into()),
        ("PROXENOS_OPERATOR_TOKEN", "operator-token-0123".into()),
        ("PROXENOS_RELAY_IDS", " r1, ,r2".into()),
        ("PROXENOS_NOTARY_SENDER_ID", "@notary:switch".into()),
    ])
}

fn load(env: &HashMap<&'static str, String>) -> Result<Config, ConfigError> {
    Config::from_lookup(|k| env.get(k).cloned())
}

#[test]
fn loads_with_defaults_and_redacts_secrets() {
    let c = load(&full()).unwrap();
    assert_eq!(c.addr.to_string(), "127.0.0.1:8080");
    assert_eq!(c.public_url, "http://127.0.0.1:8080");
    assert_eq!(c.relay_ids, ["r1", "r2"]);
    assert!(!c.sign_cards);
    let debug = format!("{c:?}");
    for secret in [
        "agent-secret",
        "gateway-secret",
        "operator-token-0123",
        &"ab".repeat(32),
    ] {
        assert!(!debug.contains(secret), "{secret} leaked");
    }
}

#[test]
fn lists_every_missing_and_malformed_variable() {
    let mut env = full();
    env.remove("SWITCH_URL");
    env.remove("SWITCH_BRIDGE_ID");
    env.insert("PROXENOS_VAULT_KEY", "12".into());
    env.insert("PROXENOS_SIGN_CARDS", "maybe".into());
    let ConfigError(problems) = load(&env).unwrap_err();
    assert_eq!(problems.len(), 4, "{problems:?}");
    env.remove("SWITCH_AGENT_KEY");
    let ConfigError(problems) = load(&env).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("proxenos register")));
    assert!(problems.iter().any(|p| p.starts_with("SWITCH_URL")));
    assert!(problems.iter().any(|p| p.starts_with("SWITCH_BRIDGE_ID")));
    assert!(problems.iter().any(|p| p.contains("64 hex")));
    env.remove("PROXENOS_NOTARY_SENDER_ID");
    let ConfigError(problems) = load(&env).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("first live event")));
    env.remove("PROXENOS_OPERATOR_ID");
    let ConfigError(problems) = load(&env).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("display name")));
    assert!(
        !problems.iter().any(|p| p.contains("12")),
        "values are not echoed"
    );
}

#[test]
fn max_rounds_defaults_to_eight_and_rejects_out_of_range() {
    let mut env = full();
    assert_eq!(load(&env).unwrap().max_rounds, 8);
    env.insert("PROXENOS_MAX_ROUNDS", "12".into());
    assert_eq!(load(&env).unwrap().max_rounds, 12);
    for bad in ["1", "51", "eight"] {
        env.insert("PROXENOS_MAX_ROUNDS", bad.into());
        let ConfigError(problems) = load(&env).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("PROXENOS_MAX_ROUNDS"))
        );
    }
}
