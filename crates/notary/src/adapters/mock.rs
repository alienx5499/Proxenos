//! Test-only HTTP mock: records requests, answers from a route function; `/events` streams
//! one `connection_state` frame and holds the socket open.

use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use futures_util::stream;

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
    }
}

type Route = dyn Fn(&str, &str) -> (u16, String) + Send + Sync;

#[derive(Clone)]
pub struct Mock {
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub base: String,
    /// Frames sent once, after `connection_state`, on the first `scope=all` stream.
    pub all_frames: Arc<Mutex<Option<String>>>,
}

impl Mock {
    /// `route(method, path)` gives status and JSON body for every non-stream request.
    pub async fn start(
        route: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        Self::streaming(String::new(), route).await
    }

    /// Like `start`, and the first `scope=all` stream also carries `frames` (SSE text).
    pub async fn streaming(
        frames: String,
        route: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mock = Self {
            seen: Arc::default(),
            base,
            all_frames: Arc::new(Mutex::new(Some(frames))),
        };
        let route: Arc<Route> = Arc::new(route);
        let m = mock.clone();
        let app = axum::Router::new()
            .fallback(move |req: Request<Body>| handle(m.clone(), route.clone(), req));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        mock
    }

    pub fn seen_path(&self, path: &str) -> Vec<Seen> {
        let seen = self.seen.lock().unwrap();
        seen.iter().filter(|s| s.path == path).cloned().collect()
    }
}

async fn handle(mock: Mock, route: Arc<Route>, req: Request<Body>) -> Response {
    let (parts, body) = req.into_parts();
    let scope_all = parts.uri.query().unwrap_or_default().contains("scope=all");
    let seen = Seen {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        headers: parts.headers,
        body: to_bytes(body, usize::MAX).await.unwrap().to_vec(),
    };
    let (method, path) = (seen.method.clone(), seen.path.clone());
    mock.seen.lock().unwrap().push(seen);
    if path.ends_with("/events") {
        let frame = "event: connection_state\ndata: {\"connection_id\":\"c1\",\"agent_id\":\"ag1\",\
                     \"generation\":1,\"scope\":\"single\",\"filter\":\"addressed\",\"rooms\":[\"r1\"],\
                     \"cursor\":0,\"heartbeat_interval_seconds\":2}\n\n";
        let extra = if scope_all {
            mock.all_frames.lock().unwrap().take().unwrap_or_default()
        } else {
            String::new()
        };
        let text = format!("{frame}{extra}");
        let first = stream::iter([Ok::<_, std::io::Error>(text.into_bytes())]);
        let body = Body::from_stream(futures_util::StreamExt::chain(first, stream::pending()));
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(body)
            .unwrap();
    }
    let (code, body) = route(&method, &path);
    Response::builder()
        .status(StatusCode::from_u16(code).unwrap())
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
