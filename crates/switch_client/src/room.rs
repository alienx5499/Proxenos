//! One-shot single-room connection for room-bound ops; its event stream belongs to the caller while open.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::stream::{Item, StreamSpec, Subscription};
use crate::{Error, SwitchClient};
use futures_util::StreamExt;

const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// A `scope=single&rooms=<room>&filter=addressed` connection. Opening it takes the
/// room over from any `scope=all` stream, so the room's events arrive on `events()`
/// until `close()`. Each one counts against the agent's 32-connection cap; dropping it
/// without `close()` releases the room best-effort, and the server sweeps the
/// connection itself once its 6 s heartbeat TTL lapses.
pub struct RoomSession {
    client: SwitchClient,
    room_id: String,
    events: Subscription,
    closed: bool,
}

#[derive(Deserialize)]
struct DocumentId {
    document_id: String,
}

impl SwitchClient {
    /// Opens the connection and waits for its `connection_state`, so the claim is in place.
    pub async fn open_room(&self, room_id: &str) -> Result<RoomSession, Error> {
        let mut events = self.subscribe(StreamSpec::room_addressed(room_id));
        let first = tokio::time::timeout(OPEN_TIMEOUT, events.next())
            .await
            .map_err(|_| Error::Protocol(format!("room {room_id} did not open within 10 s")))?;
        match first {
            Some(Ok(Item::Connected(_))) => {}
            Some(Err(e)) => return Err(e),
            Some(Ok(_)) | None => {
                return Err(Error::Protocol(
                    "stream opened without connection_state".into(),
                ));
            }
        }
        Ok(RoomSession {
            client: self.clone(),
            room_id: room_id.to_owned(),
            events,
            closed: false,
        })
    }
}

impl RoomSession {
    pub fn events(&mut self) -> &mut Subscription {
        &mut self.events
    }

    pub async fn op(&self, name: &str, args: &Value) -> Result<Value, Error> {
        self.client
            .op(name, args, Some(self.events.connection_id()))
            .await
    }

    pub async fn create_room_document(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
        content: &str,
    ) -> Result<String, Error> {
        let args = json!({
            "name": name, "description": description,
            "instructions": instructions, "content": content,
        });
        let v = self.op("create_room_document", &args).await?;
        Ok(serde_json::from_value::<DocumentId>(v)?.document_id)
    }

    pub async fn update_room_document(
        &self,
        document_id: &str,
        content: &str,
    ) -> Result<(), Error> {
        let args = json!({ "document_id": document_id, "content": content });
        self.op("update_room_document", &args).await.map(drop)
    }

    /// Prepends `@name` per target. `everyone` pages every person and wakes no agent.
    pub async fn send_targeted_message(
        &self,
        body: &str,
        target_names: &[&str],
    ) -> Result<Value, Error> {
        self.op(
            "send_targeted_message",
            &json!({ "body": body, "target_names": target_names }),
        )
        .await
    }

    /// Releases the room back to any `scope=all` connection and drops the stream.
    pub async fn close(mut self) -> Result<(), Error> {
        self.closed = true;
        release(&self.client, self.events.connection_id(), &self.room_id).await
    }
}

impl Drop for RoomSession {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (client, conn, room) = (
            self.client.clone(),
            self.events.connection_id().to_owned(),
            self.room_id.clone(),
        );
        rt.spawn(async move {
            let _ = release(&client, &conn, &room).await;
        });
    }
}

async fn release(client: &SwitchClient, connection_id: &str, room_id: &str) -> Result<(), Error> {
    let body = json!({ "connection_id": connection_id, "room_id": room_id });
    let url = client.agent_url("/connection/unsubscribe");
    client.post_json::<Value>(url, &body).await.map(drop)
}
