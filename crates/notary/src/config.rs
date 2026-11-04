//! Notary settings from the environment; every missing or malformed variable is reported at once.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use proxenos_domain::deal::DEFAULT_MAX_ROUNDS;

use crate::crypto::from_hex;

/// A secret value; Debug never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub vault_key: Secret,
    pub secret_key: Secret,
    /// `PROXENOS_RECEIPT_KEY` (64 hex); else the seed in `receipt_key_file`, created if absent.
    pub receipt_key: Option<Secret>,
    pub receipt_key_file: PathBuf,
    pub switch_url: String,
    pub agent_id: String,
    pub agent_key: Secret,
    /// Server base; the Gateway is under `/gateway`.
    pub gateway_url: String,
    pub gateway_key: Secret,
    pub bridge_id: String,
    /// The operator's display name as Switch carries it in a kickoff's `on_behalf_of`
    /// (for example `user`), not a user id; compared exactly.
    pub operator_id: String,
    /// Bearer token for the operator HTTP endpoints.
    pub operator_token: Secret,
    pub db: PathBuf,
    pub addr: SocketAddr,
    /// Base for seal links; defaults to `http://<addr>`.
    pub public_url: String,
    pub sign_cards: bool,
    /// Relay participant ids that may never negotiate.
    pub relay_ids: Vec<String>,
    /// The `sender` id Switch puts on the notary's own messages (not the agent store id).
    pub notary_sender: String,
    /// `PROXENOS_ALLOW_HTTP_GATEWAY`: accept a plain-http Gateway URL off loopback.
    pub allow_http_gateway: bool,
    /// The notary's agent name in rooms (never woken as a negotiator).
    pub notary_name: String,
    /// Lifetime of every deal, kickoff or HTTP, in seconds.
    pub deal_secs: i64,
    /// `PROXENOS_MAX_ROUNDS`: round cap for kickoff deals and HTTP deals that omit one.
    pub max_rounds: u16,
}

/// Shown when the notary's sender id is missing: it differs from the agent store id.
pub const SENDER_HINT: &str = "PROXENOS_NOTARY_SENDER_ID is not set: it is the `sender` field \
     Switch puts on the notary agent's own messages, not SWITCH_AGENT_ID; post one message as \
     the notary and copy `sender` from the first live event that carries it";

/// Shown when the notary's Switch credentials are missing: registration is a one-time step.
pub const REGISTER_HINT: &str = "register the notary once with `proxenos register` (a \
     registration key minted by the operator, always_on profile) and set SWITCH_AGENT_ID and \
     SWITCH_AGENT_KEY from its output file";

/// Shown when the operator name is missing: kickoffs carry a display name, not an id.
pub const OPERATOR_HINT: &str = "PROXENOS_OPERATOR_ID is not set: it is the operator's display \
     name exactly as Switch carries it in a kickoff's `on_behalf_of` (for example `user`), \
     not a user id";

#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub Vec<String>);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "configuration: {}", self.0.join("; "))
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut problems = Vec::new();
        let p = &mut problems;
        let vault_key = need(&get, p, "PROXENOS_VAULT_KEY");
        let secret_key = need(&get, p, "PROXENOS_SECRET_KEY");
        let switch_url = need(&get, p, "SWITCH_URL");
        let agent_id = need(&get, p, "SWITCH_AGENT_ID");
        let agent_key = need(&get, p, "SWITCH_AGENT_KEY");
        let gateway_url = need(&get, p, "SWITCH_GATEWAY_URL");
        let gateway_key = need(&get, p, "SWITCH_GATEWAY_KEY");
        let bridge_id = need(&get, p, "SWITCH_BRIDGE_ID");
        let operator_token = need(&get, p, "PROXENOS_OPERATOR_TOKEN");
        let receipt_key = get("PROXENOS_RECEIPT_KEY");
        check_hex_keys(p, &vault_key, &secret_key, receipt_key.as_deref());
        let addr = addr(&get, p);
        let sign_cards = flag(&get, p, "PROXENOS_SIGN_CARDS");
        let allow_http_gateway = flag(&get, p, "PROXENOS_ALLOW_HTTP_GATEWAY");
        let notary_sender = required(&get, p, "PROXENOS_NOTARY_SENDER_ID", || SENDER_HINT.into());
        let operator_id = required(&get, p, "PROXENOS_OPERATOR_ID", || OPERATOR_HINT.into());
        if agent_id.is_empty() || agent_key.is_empty() {
            p.push(REGISTER_HINT.into());
        }
        let deal_secs = deal_secs(&get, p);
        let max_rounds = max_rounds(&get, p);
        if !operator_token.is_empty() && operator_token.len() < 16 {
            p.push("PROXENOS_OPERATOR_TOKEN must be at least 16 characters".into());
        }
        if !problems.is_empty() {
            return Err(ConfigError(problems));
        }
        let public_url = get("PROXENOS_PUBLIC_URL").unwrap_or_else(|| format!("http://{addr}"));
        Ok(Self {
            vault_key: Secret(vault_key),
            secret_key: Secret(secret_key),
            receipt_key: receipt_key.map(Secret),
            receipt_key_file: get("PROXENOS_RECEIPT_KEY_FILE")
                .unwrap_or_else(|| "proxenos-receipt.key".into())
                .into(),
            switch_url,
            notary_sender,
            allow_http_gateway,
            notary_name: get("PROXENOS_NOTARY_NAME").unwrap_or_else(|| "notary".into()),
            deal_secs,
            max_rounds,
            agent_id,
            agent_key: Secret(agent_key),
            gateway_url,
            gateway_key: Secret(gateway_key),
            bridge_id,
            operator_id,
            operator_token: Secret(operator_token),
            db: get("PROXENOS_DB")
                .unwrap_or_else(|| "proxenos.db".into())
                .into(),
            addr,
            public_url: public_url.trim_end_matches('/').to_owned(),
            sign_cards,
            relay_ids: relay_ids(&get),
        })
    }
}

type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The variable's value, or an empty string once `problem` is noted.
fn required(
    get: Lookup<'_>,
    problems: &mut Vec<String>,
    name: &str,
    problem: impl FnOnce() -> String,
) -> String {
    get(name).unwrap_or_else(|| {
        problems.push(problem());
        String::new()
    })
}

fn need(get: Lookup<'_>, problems: &mut Vec<String>, name: &str) -> String {
    required(get, problems, name, || format!("{name} is not set"))
}

fn check_hex_keys(
    problems: &mut Vec<String>,
    vault_key: &str,
    secret_key: &str,
    receipt_key: Option<&str>,
) {
    for (name, value) in [
        ("PROXENOS_VAULT_KEY", vault_key),
        ("PROXENOS_SECRET_KEY", secret_key),
    ] {
        if !value.is_empty() && from_hex::<32>(value).is_none() {
            problems.push(format!("{name} must be 64 hex characters"));
        }
    }
    if receipt_key.is_some_and(|k| from_hex::<32>(k).is_none()) {
        problems.push("PROXENOS_RECEIPT_KEY must be 64 hex characters".into());
    }
}

fn addr(get: Lookup<'_>, problems: &mut Vec<String>) -> SocketAddr {
    let addr_text = get("PROXENOS_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into());
    addr_text.parse().unwrap_or_else(|_| {
        problems.push(format!("PROXENOS_ADDR is not host:port: {addr_text}"));
        SocketAddr::from(([127, 0, 0, 1], 8080))
    })
}

fn flag(get: Lookup<'_>, problems: &mut Vec<String>, name: &str) -> bool {
    match get(name).as_deref() {
        None | Some("0" | "false" | "no") => false,
        Some("1" | "true" | "yes") => true,
        Some(other) => {
            problems.push(format!("{name} must be true or false, not {other}"));
            false
        }
    }
}

fn deal_secs(get: Lookup<'_>, problems: &mut Vec<String>) -> i64 {
    match get("PROXENOS_DEAL_SECS") {
        None => 86_400,
        Some(v) => v.parse().ok().filter(|n| *n > 0).unwrap_or_else(|| {
            problems.push(format!(
                "PROXENOS_DEAL_SECS must be a positive number of seconds, not {v}"
            ));
            0
        }),
    }
}

fn max_rounds(get: Lookup<'_>, problems: &mut Vec<String>) -> u16 {
    match get("PROXENOS_MAX_ROUNDS") {
        None => DEFAULT_MAX_ROUNDS,
        Some(v) => v
            .parse()
            .ok()
            .filter(|n| (2..=50).contains(n))
            .unwrap_or_else(|| {
                problems.push(format!("PROXENOS_MAX_ROUNDS must be 2 to 50, not {v}"));
                0
            }),
    }
}

fn relay_ids(get: Lookup<'_>) -> Vec<String> {
    get("PROXENOS_RELAY_IDS")
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
