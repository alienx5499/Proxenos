//! POST /agents/{id}/connection/beat every 2 s; 404/409 means reopen, 409 `taken_over` means stop.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{BEAT_TIMEOUT, Error, SwitchClient};

pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);
const MIN_INTERVAL: Duration = Duration::from_millis(100);
const MAX_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Beat {
    #[serde(default)]
    pub rooms: Vec<String>,
    pub cursor: Option<i64>,
}

impl SwitchClient {
    /// One beat. `generation` comes from the stream's `connection_state` frame.
    /// A 409 with code `taken_over` comes back as `Error::TakenOver`.
    pub async fn beat(&self, connection_id: &str, generation: Option<u64>) -> Result<Beat, Error> {
        let body = json!({ "connection_id": connection_id, "generation": generation });
        let req = self
            .http
            .post(self.agent_url("/connection/beat"))
            .json(&body);
        match self.call_within(req, BEAT_TIMEOUT).await {
            Err(Error::Status { status: 409, body })
                if refusal_code(&body).as_deref() == Some("taken_over") =>
            {
                Err(Error::TakenOver)
            }
            other => other,
        }
    }
}

/// `{"detail": {"code": "...", "message": "..."}}` as FastAPI writes it.
fn refusal_code(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("detail")?.get("code")?.as_str().map(str::to_owned)
}

/// True for the answers that mean "reopen the stream", never "back off".
pub fn means_reopen(err: &Error) -> bool {
    matches!(err.status(), Some(404 | 409))
}

/// Beats until the server refuses the connection, then returns why: a 404/409
/// (reopen) or `Error::TakenOver` (stop). Transient failures keep beating;
/// the stream itself reports a lapsed TTL. The first beat goes one interval in.
pub async fn run(
    client: &SwitchClient,
    connection_id: &str,
    generation: Option<u64>,
    interval: Duration,
) -> Error {
    let interval = clamp(interval);
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match client.beat(connection_id, generation).await {
            Err(e @ Error::TakenOver) => return e,
            Err(e) if means_reopen(&e) => return e,
            _ => {}
        }
    }
}

/// The interval the server asked for, clamped to 100 ms..5 s, else the protocol default.
pub fn interval_from(seconds: Option<f64>) -> Duration {
    seconds
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| clamp(Duration::from_secs_f64(s.min(60.0))))
        .unwrap_or(DEFAULT_INTERVAL)
}

fn clamp(d: Duration) -> Duration {
    d.clamp(MIN_INTERVAL, MAX_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_is_clamped() {
        assert_eq!(interval_from(Some(1e-12)), MIN_INTERVAL);
        assert_eq!(interval_from(Some(0.0)), DEFAULT_INTERVAL);
        assert_eq!(interval_from(Some(f64::NAN)), DEFAULT_INTERVAL);
        assert_eq!(interval_from(Some(1e9)), MAX_INTERVAL);
        assert_eq!(interval_from(Some(2.0)), DEFAULT_INTERVAL);
    }

    #[test]
    fn taken_over_code_is_read_from_the_body() {
        let body = r#"{"detail":{"code":"taken_over","message":"x"}}"#;
        assert_eq!(refusal_code(body).as_deref(), Some("taken_over"));
        assert_eq!(refusal_code(r#"{"detail":"gone"}"#), None);
    }
}
