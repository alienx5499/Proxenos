//! SSE on /agents/{id}/events; resumes with Last-Event-ID and reopens on evicted or a dead beat.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use crate::event::*;
use crate::session::supervise;
use crate::{Error, SwitchClient, enc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSpec {
    /// `true` = `scope=all` (every room the agent belongs to), else `scope=single`.
    pub all_rooms: bool,
    /// `true` = `filter=addressed`.
    pub addressed_only: bool,
    /// Declared on the open URL (`rooms=`); takes those rooms over unconditionally.
    pub rooms: Vec<String>,
}

impl StreamSpec {
    pub fn all_addressed() -> Self {
        Self {
            all_rooms: true,
            addressed_only: true,
            rooms: Vec::new(),
        }
    }

    pub fn room_addressed(room_id: &str) -> Self {
        Self {
            all_rooms: false,
            addressed_only: true,
            rooms: vec![room_id.to_owned()],
        }
    }
}

/// A self-healing event stream. Yields `Ok(Item)`; an `Err` is terminal only if
/// the stream then ends (`TakenOver`, or the open was refused with a 4xx).
pub struct Subscription {
    connection_id: String,
    rx: mpsc::Receiver<Result<Item, Error>>,
    task: JoinHandle<()>,
}

impl Subscription {
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }
}

impl Stream for Subscription {
    type Item = Result<Item, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SwitchClient {
    /// Opens a stream on a fresh connection id. Needs a tokio runtime.
    pub fn subscribe(&self, spec: StreamSpec) -> Subscription {
        self.subscribe_as(spec, uuid::Uuid::new_v4().to_string(), None)
    }

    /// Opens a stream on a known connection id, resuming after `last_event_id`.
    pub fn subscribe_as(
        &self,
        spec: StreamSpec,
        connection_id: String,
        last_event_id: Option<i64>,
    ) -> Subscription {
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(supervise(
            self.clone(),
            spec,
            connection_id.clone(),
            last_event_id,
            tx,
        ));
        Subscription {
            connection_id,
            rx,
            task,
        }
    }

    pub(crate) fn events_url(&self, spec: &StreamSpec, connection_id: &str) -> String {
        let scope = if spec.all_rooms { "all" } else { "single" };
        let filter = if spec.addressed_only {
            "addressed"
        } else {
            "all"
        };
        let mut url = self.agent_url(&format!(
            "/events?connection_id={}&scope={scope}&filter={filter}",
            enc(connection_id)
        ));
        if !spec.rooms.is_empty() {
            let rooms: Vec<String> = spec.rooms.iter().map(|r| enc(r)).collect();
            url.push_str("&rooms=");
            url.push_str(&rooms.join(","));
        }
        url
    }
}
