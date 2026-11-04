//! Typed frames of the agent event stream (field names as Switch sends them).

use serde::Deserialize;
use serde_json::Value;

/// What a subscription yields.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The first frame of every (re)opened stream: the server's view, not an echo.
    Connected(ConnectionState),
    Event(Event),
    /// Events were lost; re-read room history before acting on the next event.
    Gap(Gap),
    /// The covered room set changed, e.g. a room went dark because a sibling connection claimed it.
    SubscriptionChanged(SubscriptionChanged),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SubscriptionChanged {
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ConnectionState {
    pub connection_id: String,
    pub agent_id: String,
    /// Incarnation of this connection id; sent back on every beat.
    pub generation: Option<u64>,
    pub scope: String,
    pub filter: String,
    #[serde(default)]
    pub rooms: Vec<String>,
    pub cursor: Option<i64>,
    pub heartbeat_interval_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Gap {
    pub from_sequence: Option<i64>,
    /// The cursor the stream continues from.
    pub resumed_at: Option<i64>,
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Evicted {
    pub code: Option<String>,
    #[serde(default)]
    pub reason: String,
}

/// A domain event: `message`, `command`, `room_join`, or anything newer as `Other`.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub sequence: Option<i64>,
    pub room_id: String,
    pub bridge_id: Option<String>,
    pub channel_type: Option<String>,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Message(Message),
    Command(Command),
    RoomJoin(RoomJoin),
    Other { kind: String, payload: Value },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Message {
    pub addressed: bool,
    /// Stable participant id (`@<localpart>:<server>`); opaque, match on it.
    pub sender: String,
    pub sender_name: String,
    /// `user`, `agent` or `platform`; absent on old rows.
    pub sender_kind: Option<String>,
    /// For a platform message: the person whose authority it carries.
    pub on_behalf_of: Option<String>,
    pub message_id: String,
    pub body: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub sender_role: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mimetype: String,
    pub size: u64,
    pub mxc: String,
    pub msgtype: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Command {
    pub command: String,
    #[serde(default)]
    pub args: String,
    pub user_id: String,
    pub user_name: String,
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RoomJoin {
    pub member: String,
    pub member_name: String,
    pub timestamp: i64,
    pub listening: bool,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    room_id: String,
    bridge_id: Option<String>,
    channel_type: Option<String>,
    #[serde(default)]
    payload: Value,
    sequence: Option<i64>,
}

impl Event {
    /// A known kind whose payload does not match degrades to `Other`, never an error.
    pub fn parse(data: &str) -> Result<Self, serde_json::Error> {
        let e: Envelope = serde_json::from_str(data)?;
        let typed = |p: &Value| -> Option<EventKind> {
            match e.kind.as_str() {
                "message" => serde_json::from_value(p.clone())
                    .ok()
                    .map(EventKind::Message),
                "command" => serde_json::from_value(p.clone())
                    .ok()
                    .map(EventKind::Command),
                "room_join" => serde_json::from_value(p.clone())
                    .ok()
                    .map(EventKind::RoomJoin),
                _ => None,
            }
        };
        let kind = typed(&e.payload).unwrap_or_else(|| EventKind::Other {
            kind: e.kind.clone(),
            payload: e.payload.clone(),
        });
        Ok(Self {
            sequence: e.sequence,
            room_id: e.room_id,
            bridge_id: e.bridge_id,
            channel_type: e.channel_type,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_envelope_parses_real_shape() {
        let data = r#"{"type":"message","room_id":"r1","bridge_id":null,"channel_type":"channel_public",
            "payload":{"addressed":true,"sender":"@switch-user-ab12:switch","sender_name":"asha",
            "sender_kind":"platform","on_behalf_of":"Asha K","message_id":"m1","body":"@notary status",
            "timestamp":1000,"thread_id":null,"attachments":[],"opening_message":null,"sender_role":"owner"},
            "sequence":4813,"missed":{"count":0,"reason":null}}"#;
        let e = Event::parse(data).unwrap();
        assert_eq!(e.sequence, Some(4813));
        let EventKind::Message(m) = e.kind else {
            panic!("not a message")
        };
        assert!(m.addressed);
        assert_eq!(m.on_behalf_of.as_deref(), Some("Asha K"));
        assert_eq!(m.body, "@notary status");
    }

    #[test]
    fn unknown_or_malformed_payload_degrades_to_other() {
        let e = Event::parse(r#"{"type":"message","room_id":"r","payload":{"x":1}}"#).unwrap();
        assert!(matches!(e.kind, EventKind::Other { ref kind, .. } if kind == "message"));
        let t = Event::parse(r#"{"type":"task_update","room_id":"r","payload":{}}"#).unwrap();
        assert!(matches!(t.kind, EventKind::Other { .. }));
        assert!(Event::parse("not json").is_err());
    }
}
