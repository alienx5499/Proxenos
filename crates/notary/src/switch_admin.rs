//! SwitchAdminPort over the Switch Gateway, as the operator: the notary's addressing policy
//! and the group and user lookups that feed its party rule.

use std::time::Duration;

use proxenos_app::ports::{SwitchAdminPort, SwitchError};
use serde_json::{Map, Value, json};

pub struct GatewayAdmin {
    http: reqwest::Client,
    /// Server base; the Gateway is mounted under `/gateway`.
    base: String,
    key: String,
    notary_agent_id: String,
    bridge_id: String,
    /// Admits are read-modify-write on one policy; this serializes them within the process.
    admit_lock: tokio::sync::Mutex<()>,
}

/// The operator key travels to this URL, so it must be https unless the host is loopback
/// or the operator opted in to plain http (`allow_http`).
fn check_base(base: &str, allow_http: bool) -> Result<(), SwitchError> {
    let url = reqwest::Url::parse(base).map_err(|e| SwitchError(format!("gateway url: {e}")))?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if url.scheme() == "https" || (url.scheme() == "http" && (loopback || allow_http)) {
        Ok(())
    } else {
        Err(SwitchError(
            "gateway url must be https (http only on loopback, or with \
             PROXENOS_ALLOW_HTTP_GATEWAY=1)"
                .into(),
        ))
    }
}

impl GatewayAdmin {
    pub fn new(
        base: &str,
        key: &str,
        notary_agent_id: &str,
        bridge_id: &str,
        allow_http: bool,
    ) -> Result<Self, SwitchError> {
        check_base(base, allow_http)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SwitchError(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
            notary_agent_id: notary_agent_id.to_owned(),
            bridge_id: bridge_id.to_owned(),
            admit_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// The key goes as a bearer token and as the session cookie: the Gateway's routes
    /// read the `switch_auth` cookie, so an operator session JWT works in this slot too.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, SwitchError> {
        let resp = req
            .bearer_auth(&self.key)
            .header("cookie", format!("switch_auth={}", self.key))
            .send()
            .await
            .map_err(|e| SwitchError(format!("gateway: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(SwitchError(format!("gateway answered {status}: {text}")));
        }
        serde_json::from_str(&text).map_err(|e| SwitchError(format!("gateway json: {e}")))
    }

    fn url(&self, path: &str) -> String {
        format!("{}/gateway{path}", self.base)
    }

    async fn policy(&self) -> Result<Value, SwitchError> {
        let path = format!("/agents/{}", self.notary_agent_id);
        let summary = self.send(self.http.get(self.url(&path))).await?;
        Ok(summary
            .get("addressing_policy")
            .cloned()
            .unwrap_or(Value::Null))
    }
}

/// The party rule for one deal group.
pub fn party_rule(group: &str, users: &[String], agents: &[String]) -> Value {
    json!({
        "rooms": "*",
        "room_groups": [group],
        "users": users,
        "agents": agents,
        "owner": false,
        "owner_agents": false,
        "platform": false,
    })
}

/// A rule with Switch's defaults filled in and id lists sorted, for content comparison.
fn normalized(rule: &Value) -> Value {
    let mut out = Map::new();
    for dim in ["rooms", "room_groups", "users", "agents"] {
        let v = match rule.get(dim) {
            Some(Value::Array(ids)) => {
                let mut ids = ids.clone();
                ids.sort_by_key(|v| v.to_string());
                Value::Array(ids)
            }
            Some(v) if !v.is_null() => v.clone(),
            _ => json!("*"),
        };
        out.insert(dim.into(), v);
    }
    for flag in ["owner", "owner_agents", "platform"] {
        let v = rule.get(flag).and_then(Value::as_bool).unwrap_or(false);
        out.insert(flag.into(), v.into());
    }
    Value::Object(out)
}

/// The rules after admitting `rule`, or `None` when an equal rule is already there.
/// An open policy (null or no rules) starts from the owner rule, so the operator keeps access.
pub fn with_rule(policy: &Value, rule: Value) -> Option<Vec<Value>> {
    let mut rules = match policy.get("rules") {
        Some(Value::Array(r)) if !r.is_empty() => r.clone(),
        _ => vec![json!({"owner": true, "users": [], "agents": []})],
    };
    let want = normalized(&rule);
    if rules.iter().any(|r| normalized(r) == want) {
        return None;
    }
    rules.push(rule);
    Some(rules)
}

/// Exact name, else the one group whose name is `<name> <anything>` (names carry the deal id).
pub fn pick_group(groups: &Value, name: &str) -> Result<String, SwitchError> {
    let list = groups.as_array().map(Vec::as_slice).unwrap_or_default();
    fn named<'a>(g: &&'a Value) -> Option<&'a str> {
        g.get("name").and_then(Value::as_str)
    }
    let id = |g: &Value| g.get("id").and_then(Value::as_str).map(str::to_owned);
    if let Some(g) = list.iter().find(|g| named(g) == Some(name)) {
        return id(g).ok_or_else(|| SwitchError(format!("group {name} has no id")));
    }
    let prefix = format!("{name} ");
    let matches: Vec<&Value> = list
        .iter()
        .filter(|g| named(g).is_some_and(|n| n.starts_with(&prefix)))
        .collect();
    match matches.as_slice() {
        [g] => id(g).ok_or_else(|| SwitchError(format!("group {name} has no id"))),
        [] => Err(SwitchError(format!("no room group named {name}"))),
        _ => Err(SwitchError(format!(
            "several room groups start with {name}"
        ))),
    }
}

/// By Gateway username, or by ExternalUser id already; more than one match is an error.
pub fn pick_user(users: &Value, username: &str) -> Result<String, SwitchError> {
    let ids: Vec<&str> = users
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| {
            u.get("external_username").and_then(Value::as_str) == Some(username)
                || u.get("id").and_then(Value::as_str) == Some(username)
        })
        .filter_map(|u| u.get("id").and_then(Value::as_str))
        .collect();
    match ids.as_slice() {
        [id] => Ok((*id).to_owned()),
        [] => Err(SwitchError(format!("no bridged user {username}"))),
        _ => Err(SwitchError(format!(
            "several bridged users match {username}"
        ))),
    }
}

impl SwitchAdminPort for GatewayAdmin {
    async fn admit_parties(
        &self,
        group: &str,
        users: &[String],
        agents: &[String],
    ) -> Result<(), SwitchError> {
        // Another writer (Console, a second notary) can still race the PUT;
        // the re-read catches a lost rule and one retry repairs it.
        let _guard = self.admit_lock.lock().await;
        let rule = party_rule(group, users, agents);
        let url = self.url(&format!(
            "/agents/{}/addressing-policy",
            self.notary_agent_id
        ));
        for _ in 0..2 {
            let Some(rules) = with_rule(&self.policy().await?, rule.clone()) else {
                return Ok(());
            };
            let body = json!({ "policy": { "rules": rules } });
            self.send(self.http.put(&url).json(&body)).await?;
            if with_rule(&self.policy().await?, rule.clone()).is_none() {
                return Ok(());
            }
        }
        Err(SwitchError(
            "party rule missing from the notary policy after two writes".into(),
        ))
    }

    async fn group_id(&self, name: &str) -> Result<String, SwitchError> {
        let groups = self.send(self.http.get(self.url("/room-groups"))).await?;
        pick_group(&groups, name)
    }

    async fn external_user_id(&self, username: &str) -> Result<String, SwitchError> {
        let path = format!("/collaborations/{}/users", self.bridge_id);
        let users = self.send(self.http.get(self.url(&path))).await?;
        pick_user(&users, username)
    }
}

#[cfg(test)]
#[path = "adapters/switch_admin_tests.rs"]
mod tests;
