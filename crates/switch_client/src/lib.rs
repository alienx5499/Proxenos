//! Thin Switch agent-protocol client. No domain knowledge.

pub mod approvals;
pub mod error;
mod event;
pub mod heartbeat;
pub mod ops;
pub mod register;
pub mod room;
mod session;
pub mod sse;
pub mod stream;

pub use error::Error;
pub use register::Credentials;

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Posts and roster reads: short, so a Switch outage cannot stall a caller's loop for long.
const QUICK_TIMEOUT: Duration = Duration::from_secs(5);
/// Under half the server's 6 s connection TTL.
const BEAT_TIMEOUT: Duration = Duration::from_secs(3);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// The server writes a keepalive every 15 s; silence for twice that is a dead socket.
const STREAM_IDLE: Duration = Duration::from_secs(30);

/// An agent's handle on the Switch agent bridge, authenticated by its API key.
#[derive(Clone, Debug)]
pub struct SwitchClient {
    http: reqwest::Client,
    base: String,
    creds: Credentials,
    stream_idle: Duration,
}

impl SwitchClient {
    /// `base_url` is the agent bridge (where `GET /health` answers), not the Gateway.
    pub fn new(base_url: &str, creds: Credentials) -> Result<Self, Error> {
        Ok(Self {
            http: http_client()?,
            base: base_url.trim_end_matches('/').to_owned(),
            creds,
            stream_idle: STREAM_IDLE,
        })
    }

    /// How long an open stream may stay silent before it is reopened (default 30 s).
    pub fn with_stream_idle_timeout(mut self, idle: Duration) -> Self {
        self.stream_idle = idle;
        self
    }

    pub fn agent_id(&self) -> &str {
        &self.creds.agent_id
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn agent_url(&self, rest: &str) -> String {
        self.url(&format!("/agents/{}{rest}", enc(&self.creds.agent_id)))
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.creds.api_key)
    }

    async fn call<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T, Error> {
        self.call_within(req, CALL_TIMEOUT).await
    }

    async fn call_within<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<T, Error> {
        let resp = self.authed(req).timeout(timeout).send().await?;
        decode(resp).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        url: String,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        self.call(self.http.post(url).json(body)).await
    }

    async fn get_json<T: DeserializeOwned>(&self, url: String) -> Result<T, Error> {
        self.call(self.http.get(url)).await
    }
}

/// Percent-encodes one path segment or query value (RFC 3986 unreserved kept).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn http_client() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

/// Non-2xx becomes `Error::Status` with the body kept for the operator.
async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, Error> {
    let resp = check(resp).await?;
    let bytes = resp.bytes().await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response, Error> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Status {
        status: status.as_u16(),
        body,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn enc_escapes_separators() {
        assert_eq!(super::enc("a&b=c/d e,f"), "a%26b%3Dc%2Fd%20e%2Cf");
        assert_eq!(super::enc("D-0007_x.y~"), "D-0007_x.y~");
    }
}
