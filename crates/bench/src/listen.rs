//! Both seats' event streams, opened and confirmed before the deal opens, so the notary's
//! first wake cannot land before anyone is listening.

use std::time::Duration;

use futures_util::StreamExt;
use proxenos_switch_client::SwitchClient;
use proxenos_switch_client::stream::{Item, StreamSpec, Subscription};

use crate::BenchError;

/// Opens an addressed stream per client and waits for each server's first `Connected` frame.
pub async fn listen(
    clients: [&SwitchClient; 2],
    wait: Duration,
) -> Result<[Subscription; 2], BenchError> {
    let mut subs = clients.map(|c| c.subscribe(StreamSpec::all_addressed()));
    let [b, s] = &mut subs;
    let both = async { tokio::join!(connected(b), connected(s)) };
    match tokio::time::timeout(wait, both).await {
        Ok((true, true)) => Ok(subs),
        _ => Err(BenchError::Http(format!(
            "event streams not connected within {wait:?}"
        ))),
    }
}

/// The first frame of a fresh stream is `Connected`; anything else means it is not usable.
async fn connected(sub: &mut Subscription) -> bool {
    matches!(sub.next().await, Some(Ok(Item::Connected(_))))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use axum::response::Response;
    use futures_util::stream;
    use proxenos_switch_client::Credentials;

    use super::*;

    const STATE: &str = "event: connection_state\ndata: {\"connection_id\":\"c\",\"agent_id\":\"a\",\"generation\":1,\"scope\":\"all\",\"filter\":\"addressed\",\"rooms\":[],\"cursor\":0,\"heartbeat_interval_seconds\":30}\n\n";

    /// A bridge whose `/events` sends the connected frame only when `ready` is true.
    async fn bridge(ready: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().fallback(move |_: Request<Body>| async move {
            let first: Vec<Result<Vec<u8>, std::io::Error>> = if ready {
                vec![Ok(STATE.as_bytes().to_vec())]
            } else {
                vec![]
            };
            let body = Body::from_stream(stream::iter(first).chain(stream::pending()));
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(body)
                .unwrap()
        });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    fn client(base: &str, id: &str) -> SwitchClient {
        let creds = Credentials {
            agent_id: id.into(),
            api_key: "k".into(),
        };
        SwitchClient::new(base, creds).unwrap()
    }

    #[tokio::test]
    async fn returns_once_both_streams_are_connected() {
        let base = bridge(true).await;
        let (b, s) = (client(&base, "b"), client(&base, "s"));
        assert!(listen([&b, &s], Duration::from_secs(5)).await.is_ok());
    }

    #[tokio::test]
    async fn fails_when_a_stream_never_connects() {
        let base = bridge(false).await;
        let (b, s) = (client(&base, "b"), client(&base, "s"));
        let err = listen([&b, &s], Duration::from_millis(300)).await.err();
        assert!(matches!(err, Some(BenchError::Http(m)) if m.contains("not connected")));
    }
}
