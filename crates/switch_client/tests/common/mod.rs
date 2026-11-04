//! In-process mock of the Switch agent bridge: records every request, replies from a script.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use futures_util::stream;
use proxenos_switch_client::{Credentials, SwitchClient};

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
    pub at: std::time::Instant,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    pub fn param(&self, key: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == key).then(|| v.to_owned())
        })
    }
}

/// One scripted SSE response: frames, then either end or hold the socket open.
pub struct Sse {
    pub body: String,
    pub hold_open: bool,
}

#[derive(Default)]
pub struct Script {
    pub sse: VecDeque<Sse>,
    pub beat_status: u16,
    /// Non-zero: the events route answers this status instead of streaming.
    pub open_status: u16,
    /// Body for a non-200 beat.
    pub beat_body: Option<String>,
    /// Reply for any non-SSE request: status, JSON body.
    pub reply: Option<(u16, String)>,
}

#[derive(Clone)]
pub struct Mock {
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub script: Arc<Mutex<Script>>,
    pub base: String,
}

impl Mock {
    pub async fn start(script: Script) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mock = Self {
            seen: Arc::default(),
            script: Arc::new(Mutex::new(script)),
            base,
        };
        let m = mock.clone();
        let app = axum::Router::new().fallback(move |req: Request<Body>| handle(m.clone(), req));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        mock
    }

    pub fn client(&self) -> SwitchClient {
        let creds = Credentials {
            agent_id: "ag1".into(),
            api_key: "key1".into(),
        };
        SwitchClient::new(&self.base, creds).unwrap()
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn seen_path(&self, path: &str) -> Vec<Seen> {
        self.seen().into_iter().filter(|s| s.path == path).collect()
    }
}

async fn handle(mock: Mock, req: Request<Body>) -> Response {
    let (parts, body) = req.into_parts();
    let seen = Seen {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().unwrap_or_default().to_owned(),
        headers: parts.headers,
        body: to_bytes(body, usize::MAX).await.unwrap().to_vec(),
        at: std::time::Instant::now(),
    };
    let path = seen.path.clone();
    mock.seen.lock().unwrap().push(seen);
    let mut script = mock.script.lock().unwrap();
    if path.ends_with("/events") {
        if script.open_status != 0 {
            return status(script.open_status, r#"{"detail":"refused"}"#);
        }
        let Some(sse) = script.sse.pop_front() else {
            return status(503, "{}");
        };
        let first = stream::iter([Ok::<_, std::io::Error>(sse.body.into_bytes())]);
        let body = if sse.hold_open {
            Body::from_stream(futures_util::StreamExt::chain(first, stream::pending()))
        } else {
            Body::from_stream(first)
        };
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(body)
            .unwrap();
    }
    if path.ends_with("/connection/beat") {
        let code = if script.beat_status == 0 {
            200
        } else {
            script.beat_status
        };
        let body = match (&script.beat_body, code) {
            (Some(b), c) if c != 200 => b.clone(),
            _ => r#"{"ok":true,"rooms":[],"cursor":0}"#.to_owned(),
        };
        return status(code, &body);
    }
    match &script.reply {
        Some((code, body)) => status(*code, body),
        None => status(200, r#"{"ok":true}"#),
    }
}

fn status(code: u16, body: &str) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(code).unwrap())
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub fn state_frame(conn: &str, interval: f64) -> String {
    format!(
        "event: connection_state\ndata: {{\"connection_id\":\"{conn}\",\"agent_id\":\"ag1\",\"generation\":3,\
         \"scope\":\"all\",\"filter\":\"addressed\",\"rooms\":[\"r1\"],\"cursor\":0,\
         \"heartbeat_interval_seconds\":{interval}}}\n\n"
    )
}

pub fn message_frame(seq: i64, body: &str) -> String {
    format!(
        "id: {seq}\nevent: message\ndata: {{\"type\":\"message\",\"room_id\":\"r1\",\"bridge_id\":null,\
         \"channel_type\":null,\"payload\":{{\"addressed\":true,\"sender\":\"@switch-user-a:x\",\
         \"sender_name\":\"asha\",\"message_id\":\"m{seq}\",\"body\":\"{body}\",\"timestamp\":1000,\
         \"thread_id\":null,\"attachments\":[]}},\"sequence\":{seq}}}\n\n"
    )
}
