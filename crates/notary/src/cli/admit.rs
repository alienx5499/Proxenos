//! `proxenos admit D-0007`: asks the running notary to admit the deal's parties.

use std::time::Duration;

use serde_json::{Map, Value, json};

use std::path::PathBuf;

use super::{CliError, read_secret};
use crate::config::Secret;

#[derive(Debug, clap::Args)]
pub struct AdmitArgs {
    pub deal: String,
    #[arg(long, env = "PROXENOS_URL", default_value = "http://127.0.0.1:8080")]
    pub url: String,
    /// File holding the operator token (`-` for stdin); else `PROXENOS_OPERATOR_TOKEN`.
    #[arg(long)]
    pub token_file: Option<PathBuf>,
    /// Room group name; defaults to the group whose name starts with the deal id.
    #[arg(long)]
    pub group: Option<String>,
    /// Gateway username of an owner; give both.
    #[arg(long = "owner", required = true, num_args = 1)]
    pub owners: Vec<String>,
    /// Agent store id of a negotiator (not its `sender` id); give both.
    #[arg(long = "agent", required = true, num_args = 1)]
    pub agents: Vec<String>,
}

pub fn body(args: &AdmitArgs) -> Value {
    let mut body = Map::new();
    if let Some(g) = &args.group {
        body.insert("group".into(), json!(g));
    }
    body.insert("owners".into(), json!(args.owners));
    body.insert("agents".into(), json!(args.agents));
    Value::Object(body)
}

/// Returns the admitted group id.
pub async fn run(args: &AdmitArgs) -> Result<String, CliError> {
    let token = read_secret("PROXENOS_OPERATOR_TOKEN", args.token_file.as_deref())?
        .ok_or_else(|| CliError("set PROXENOS_OPERATOR_TOKEN or pass --token-file".into()))?;
    send(args, &token).await
}

async fn send(args: &AdmitArgs, token: &Secret) -> Result<String, CliError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(CliError::new)?;
    let url = format!(
        "{}/api/deals/{}/admit",
        args.url.trim_end_matches('/'),
        args.deal
    );
    let resp = http
        .post(url)
        .bearer_auth(token.expose())
        .json(&body(args))
        .send()
        .await
        .map_err(|e| CliError::new(e.without_url()))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(CliError(format!(
            "notary answered {status}: {}",
            v["error"]
        )));
    }
    Ok(v["group_id"].as_str().unwrap_or_default().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn posts_admit_with_the_operator_token() {
        let mock = crate::adapters::mock::Mock::start(|_, _| {
            (200, r#"{"deal_id":"D-0007","group_id":"g7"}"#.to_owned())
        })
        .await;
        let args = AdmitArgs {
            deal: "D-0007".into(),
            url: mock.base.clone(),
            token_file: None,
            group: None,
            owners: vec!["asha".into(), "ravi".into()],
            agents: vec!["a1".into(), "a2".into()],
        };
        assert_eq!(send(&args, &Secret::new("tok")).await.unwrap(), "g7");
        let seen = &mock.seen_path("/api/deals/D-0007/admit")[0];
        assert_eq!(seen.header("authorization").unwrap(), "Bearer tok");
        assert_eq!(
            seen.json(),
            json!({"owners": ["asha", "ravi"], "agents": ["a1", "a2"]})
        );
    }
}
