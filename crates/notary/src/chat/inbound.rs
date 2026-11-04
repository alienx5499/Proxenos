//! Inbound chat message as the router sees it, mapped from a Switch stream event.

use proxenos_domain::party::{ParticipantId, RoomId};
use proxenos_switch_client::stream::{Event, EventKind, Message};

use super::canned::{self, Canned};
use super::grammar::{self, Command, ParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderKind {
    User,
    Agent,
    Platform,
    /// Absent or unrecognised; treated like any non-platform sender.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbound {
    pub message_id: String,
    pub room: RoomId,
    pub sender: ParticipantId,
    /// Display name, used only to remember which name a negotiator answers to.
    pub sender_name: String,
    pub sender_kind: SenderKind,
    pub on_behalf_of: Option<String>,
    pub body: String,
    pub addressed: bool,
}

impl Inbound {
    /// `None` for anything that is not a message, or a message with a blank id.
    pub fn from_event(event: &Event) -> Option<Self> {
        match &event.kind {
            EventKind::Message(m) => Self::from_message(&event.room_id, m),
            _ => None,
        }
    }

    pub fn from_message(room_id: &str, m: &Message) -> Option<Self> {
        if m.message_id.trim().is_empty() {
            return None;
        }
        let sender_kind = match m.sender_kind.as_deref() {
            Some("user") => SenderKind::User,
            Some("agent") => SenderKind::Agent,
            Some("platform") => SenderKind::Platform,
            _ => SenderKind::Unknown,
        };
        Some(Self {
            message_id: m.message_id.clone(),
            room: RoomId::new(room_id).ok()?,
            sender: ParticipantId::new(m.sender.as_str()).ok()?,
            sender_name: m.sender_name.clone(),
            sender_kind,
            on_behalf_of: m.on_behalf_of.clone(),
            body: m.body.clone(),
            addressed: m.addressed,
        })
    }

    /// With the `@notary` prefix, or without it: an addressed delivery (a targeted message or
    /// a template kickoff) may carry the bare command.
    pub fn command(&self) -> Result<Command, ParseError> {
        match grammar::parse(&self.body) {
            Err(ParseError::NotAddressed) => grammar::parse_command(&self.body),
            other => other,
        }
    }

    /// A Switch canned text posted as an agent (or by the platform): an addressing refusal
    /// (`recipient.py` `_post_auto_reply`), an availability auto-reply or a join greeting.
    /// A body with an `@notary` command on a later line is not dropped, so a command cannot
    /// hide behind it.
    pub fn canned(&self) -> Option<Canned> {
        if !matches!(self.sender_kind, SenderKind::Agent | SenderKind::Platform) {
            return None;
        }
        let later_command = self
            .body
            .lines()
            .skip(1)
            .any(|line| !matches!(grammar::parse(line), Err(ParseError::NotAddressed)));
        if later_command {
            return None;
        }
        canned::classify(&self.body)
    }
}
