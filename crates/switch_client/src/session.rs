//! The stream supervisor: open, read frames, beat on its own task, reopen with backoff.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::event::{ConnectionState, Event, Evicted, Gap, Item, SubscriptionChanged};
use crate::stream::StreamSpec;
use crate::{Error, SwitchClient, check, heartbeat, sse};

const MIN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(16);
/// A connection that lived this long resets the backoff.
const SURVIVED: Duration = Duration::from_secs(10);

pub(crate) type Tx = mpsc::Sender<Result<Item, Error>>;

enum End {
    Reopen,
    Closed,
    Fatal(Error),
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) async fn supervise(
    client: SwitchClient,
    spec: StreamSpec,
    conn: String,
    mut last: Option<i64>,
    tx: Tx,
) {
    let mut backoff = MIN_BACKOFF;
    loop {
        let mut connected_at = None;
        match session(&client, &spec, &conn, &mut last, &tx, &mut connected_at).await {
            End::Closed => return,
            End::Fatal(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
            End::Reopen => {}
        }
        if connected_at.is_some_and(|t: Instant| t.elapsed() >= SURVIVED) {
            backoff = MIN_BACKOFF;
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = tx.closed() => return,
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Sends the stream request; a refusal that retrying cannot fix is fatal.
async fn open_stream(
    client: &SwitchClient,
    spec: &StreamSpec,
    conn: &str,
    last: Option<i64>,
) -> Result<reqwest::Response, End> {
    let mut req = client
        .authed(client.http.get(client.events_url(spec, conn)))
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if let Some(id) = last {
        req = req.header("Last-Event-ID", id.to_string());
    }
    let Ok(resp) = req.send().await else {
        return Err(End::Reopen);
    };
    match check(resp).await {
        Ok(r) => Ok(r),
        Err(e) if matches!(e.status(), Some(s) if (400..500).contains(&s) && s != 408 && s != 429) => {
            Err(End::Fatal(e))
        }
        Err(_) => Err(End::Reopen),
    }
}

async fn session(
    client: &SwitchClient,
    spec: &StreamSpec,
    conn: &str,
    last: &mut Option<i64>,
    tx: &Tx,
    connected_at: &mut Option<Instant>,
) -> End {
    let resp = match open_stream(client, spec, conn, *last).await {
        Ok(r) => r,
        Err(end) => return end,
    };
    let mut body = resp.bytes_stream();
    let mut parser = sse::Parser::default();
    // The beat runs on its own task so a slow consumer cannot starve it.
    let (beat_tx, mut beat_rx) = mpsc::channel::<Error>(1);
    let mut _beat: Option<AbortOnDrop> = None;
    let mut deadline = Instant::now() + client.stream_idle;
    loop {
        tokio::select! {
            chunk = body.next() => {
                let bytes = match chunk {
                    Some(Ok(b)) => b,
                    None | Some(Err(_)) => return End::Reopen,
                };
                let frames = match parser.try_push(&bytes) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return End::Reopen;
                    }
                };
                for f in frames {
                    match on_frame(f, last, tx).await {
                        Step::Continue => {}
                        Step::Connected(interval, generation) => {
                            *connected_at = Some(Instant::now());
                            _beat = Some(spawn_beat(client, conn, generation, interval, beat_tx.clone()));
                        }
                        Step::End(end) => return end,
                    }
                }
                deadline = Instant::now() + client.stream_idle;
            }
            Some(e) = beat_rx.recv() => {
                return if matches!(e, Error::TakenOver) { End::Fatal(e) } else { End::Reopen };
            }
            _ = tokio::time::sleep_until(deadline) => return End::Reopen,
            _ = tx.closed() => return End::Closed,
        }
    }
}

fn spawn_beat(
    client: &SwitchClient,
    conn: &str,
    generation: Option<u64>,
    interval: Duration,
    out: mpsc::Sender<Error>,
) -> AbortOnDrop {
    let (client, conn) = (client.clone(), conn.to_owned());
    AbortOnDrop(tokio::spawn(async move {
        let e = heartbeat::run(&client, &conn, generation, interval).await;
        let _ = out.send(e).await;
    }))
}

enum Step {
    Continue,
    Connected(Duration, Option<u64>),
    End(End),
}

async fn on_frame(f: sse::RawFrame, last: &mut Option<i64>, tx: &Tx) -> Step {
    let item = match f.event.as_str() {
        // An id-only block: no dispatch, cursor update only.
        "" => {
            if let Some(seq) = f.id.as_deref().and_then(|s| s.trim().parse().ok()) {
                *last = Some(seq);
            }
            return Step::Continue;
        }
        "connection_state" => match serde_json::from_str::<ConnectionState>(&f.data) {
            Ok(s) => {
                let step = Step::Connected(
                    heartbeat::interval_from(s.heartbeat_interval_seconds),
                    s.generation,
                );
                if tx.send(Ok(Item::Connected(s))).await.is_err() {
                    return Step::End(End::Closed);
                }
                return step;
            }
            Err(_) => return Step::End(End::Reopen),
        },
        "evicted" => {
            let ev = serde_json::from_str::<Evicted>(&f.data).ok();
            return match ev.and_then(|e| e.code) {
                Some(code) if code == "taken_over" => Step::End(End::Fatal(Error::TakenOver)),
                _ => Step::End(End::Reopen),
            };
        }
        "gap" => {
            let gap = serde_json::from_str::<Gap>(&f.data).unwrap_or_else(|_| Gap {
                from_sequence: None,
                resumed_at: None,
                rooms: Vec::new(),
                reason: format!("unparseable gap frame: {}", f.data),
            });
            if gap.resumed_at.is_some() {
                *last = gap.resumed_at;
            }
            Ok(Item::Gap(gap))
        }
        "subscription_changed" => serde_json::from_str::<SubscriptionChanged>(&f.data)
            .map(Item::SubscriptionChanged)
            .map_err(Error::from),
        kind if f.id.is_some() || is_domain(kind) => {
            if let Some(seq) = f.id.as_deref().and_then(|s| s.trim().parse().ok()) {
                *last = Some(seq);
            }
            Event::parse(&f.data).map(Item::Event).map_err(Error::from)
        }
        // room_released only reaches clients declaring protocol 6+, which this one does not.
        _ => return Step::Continue,
    };
    if tx.send(item).await.is_err() {
        return Step::End(End::Closed);
    }
    Step::Continue
}

fn is_domain(kind: &str) -> bool {
    matches!(kind, "message" | "command" | "room_join") || kind.starts_with("task_")
}
