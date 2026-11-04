//! Fakes for the Switch-facing ports: they record calls and return canned answers.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use proxenos_domain::party::RoomId;

use crate::ports::{
    CardOutcome, DocId, RequestId, RosterPort, SignCard, SignatureOutcomes, SwitchAdminPort,
    SwitchError, SwitchPort,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchCall {
    Post {
        room: RoomId,
        text: String,
    },
    WriteDocument {
        room: RoomId,
        name: String,
        content: String,
    },
    UpdateDocument {
        room: RoomId,
        doc: DocId,
        content: String,
    },
    RequestSignature(SignCard),
    AckOutcome {
        request: RequestId,
    },
}

/// Records every call; card outcomes pushed by a test stay undelivered until acked.
/// With `down` set, posts and document writes fail and are not recorded.
#[derive(Default)]
pub struct FakeSwitch {
    calls: Mutex<Vec<SwitchCall>>,
    outcomes: Mutex<Vec<CardOutcome>>,
    pub down: AtomicBool,
}

impl FakeSwitch {
    pub fn calls(&self) -> Vec<SwitchCall> {
        self.calls.lock().expect("switch lock").clone()
    }

    pub fn push_outcome(&self, outcome: CardOutcome) {
        self.outcomes.lock().expect("outcomes lock").push(outcome);
    }

    fn check_up(&self) -> Result<(), SwitchError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(SwitchError("switch is down".into()));
        }
        Ok(())
    }

    /// Records `call`; returns its 1-based position, used for ids.
    fn record(&self, call: SwitchCall) -> usize {
        let mut calls = self.calls.lock().expect("switch lock");
        calls.push(call);
        calls.len()
    }
}

impl SwitchPort for FakeSwitch {
    async fn post(&self, room: &RoomId, text: &str) -> Result<(), SwitchError> {
        self.check_up()?;
        self.record(SwitchCall::Post {
            room: room.clone(),
            text: text.into(),
        });
        Ok(())
    }

    async fn write_document(
        &self,
        room: &RoomId,
        name: &str,
        content: &str,
    ) -> Result<DocId, SwitchError> {
        self.check_up()?;
        let (room, name, content) = (room.clone(), name.into(), content.into());
        let n = self.record(SwitchCall::WriteDocument {
            room,
            name,
            content,
        });
        Ok(DocId(format!("doc-{n}")))
    }

    async fn update_document(
        &self,
        room: &RoomId,
        doc: &DocId,
        content: &str,
    ) -> Result<(), SwitchError> {
        self.check_up()?;
        let (room, doc, content) = (room.clone(), doc.clone(), content.into());
        self.record(SwitchCall::UpdateDocument { room, doc, content });
        Ok(())
    }

    async fn request_signature(&self, card: &SignCard) -> Result<RequestId, SwitchError> {
        let n = self.record(SwitchCall::RequestSignature(card.clone()));
        Ok(RequestId(format!("req-{n}")))
    }

    async fn ack_outcome(&self, request: &RequestId) -> Result<(), SwitchError> {
        self.record(SwitchCall::AckOutcome {
            request: request.clone(),
        });
        let mut outcomes = self.outcomes.lock().expect("outcomes lock");
        outcomes.retain(|o| &o.request != request);
        Ok(())
    }
}

impl SignatureOutcomes for FakeSwitch {
    async fn undelivered(&self) -> Result<Vec<CardOutcome>, SwitchError> {
        Ok(self.outcomes.lock().expect("outcomes lock").clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    pub group: String,
    pub users: Vec<String>,
    pub agents: Vec<String>,
}

/// Lookups from the maps; `admitted` records each `admit_parties` call.
#[derive(Default)]
pub struct FakeAdmin {
    pub groups: HashMap<String, String>,
    pub users: HashMap<String, String>,
    pub admitted: Mutex<Vec<Admitted>>,
}

impl SwitchAdminPort for FakeAdmin {
    async fn admit_parties(
        &self,
        group: &str,
        users: &[String],
        agents: &[String],
    ) -> Result<(), SwitchError> {
        let call = Admitted {
            group: group.to_owned(),
            users: users.to_vec(),
            agents: agents.to_vec(),
        };
        self.admitted.lock().expect("admin lock").push(call);
        Ok(())
    }

    async fn group_id(&self, name: &str) -> Result<String, SwitchError> {
        let id = self.groups.get(name).cloned();
        id.ok_or_else(|| SwitchError(format!("no group {name}")))
    }

    async fn external_user_id(&self, username: &str) -> Result<String, SwitchError> {
        let id = self.users.get(username).cloned();
        id.ok_or_else(|| SwitchError(format!("no user {username}")))
    }
}

#[derive(Default)]
pub struct FakeRoster(pub HashMap<RoomId, Vec<String>>);

impl RosterPort for FakeRoster {
    async fn agent_names(&self, room: &RoomId) -> Result<Vec<String>, SwitchError> {
        Ok(self.0.get(room).cloned().unwrap_or_default())
    }
}
