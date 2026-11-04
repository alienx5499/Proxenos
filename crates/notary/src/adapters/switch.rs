//! SwitchPort, SignatureOutcomes and RosterPort over the agent-protocol client, as the notary.

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::{FutureExt, StreamExt};
use proxenos_app::ports::{
    CardAnswer, CardOutcome, DocId, RequestId, RosterPort, SignCard, SignatureOutcomes,
    SwitchError, SwitchPort,
};
use proxenos_domain::party::{ParticipantId, RoomId};
use proxenos_switch_client::SwitchClient;
use proxenos_switch_client::approvals::{ApprovalCard, ApprovalOption, ApprovalView, Decision};
use proxenos_switch_client::stream::Item;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::rfc3339;

pub const SIGN_OPTION: &str = "sign";
pub const DECLINE_OPTION: &str = "decline";

pub struct SwitchAdapter {
    client: SwitchClient,
    /// One approvals session per notary process.
    session_id: String,
    /// Where a one-shot room connection's events go while it holds the room.
    room_events: Option<mpsc::Sender<Item>>,
    /// Session of each outcome seen, so an ack reaches a card from an earlier process.
    outcome_sessions: Mutex<HashMap<String, String>>,
}

fn err(e: proxenos_switch_client::Error) -> SwitchError {
    SwitchError(e.to_string())
}

impl SwitchAdapter {
    /// `room_events` should feed the chat router: a one-shot connection takes its room
    /// over, so addressed messages sent meanwhile arrive there and nowhere else.
    pub fn new(client: SwitchClient, room_events: Option<mpsc::Sender<Item>>) -> Self {
        Self {
            client,
            session_id: uuid::Uuid::new_v4().to_string(),
            room_events,
            outcome_sessions: Mutex::default(),
        }
    }

    /// Never waits: the router may be the caller of this op, so a blocking send on a full
    /// channel would deadlock it. A full or closed channel drops the event and logs it; the
    /// sender can repeat, and redelivery is deduped on `message_id` anyway.
    fn forward(&self, item: Result<Item, proxenos_switch_client::Error>) {
        let (Ok(item), Some(tx)) = (item, &self.room_events) else {
            return;
        };
        if let Err(e) = tx.try_send(item) {
            eprintln!("switch: dropped an event from a one-shot room connection: {e}");
        }
    }

    /// Opens a single-room connection, runs one op on it while forwarding its events, closes it.
    async fn room_op(&self, room: &RoomId, name: &str, args: Value) -> Result<Value, SwitchError> {
        let mut session = self.client.open_room(room.as_str()).await.map_err(err)?;
        let conn = session.events().connection_id().to_owned();
        let result = {
            let events = session.events();
            let forward = async {
                while let Some(item) = events.next().await {
                    self.forward(item);
                }
                std::future::pending::<()>().await
            };
            tokio::select! {
                r = self.client.op(name, &args, Some(&conn)) => r,
                () = forward => unreachable!("forwarding never completes"),
            }
        };
        while let Some(Some(item)) = session.events().next().now_or_never() {
            self.forward(item);
        }
        if let Err(e) = session.close().await {
            eprintln!("switch: releasing room {room} failed: {e}");
        }
        result.map_err(err)
    }
}

impl SwitchPort for SwitchAdapter {
    async fn post(&self, room: &RoomId, text: &str) -> Result<(), SwitchError> {
        self.client
            .post_message(room.as_str(), text)
            .await
            .map(drop)
            .map_err(err)
    }

    async fn write_document(
        &self,
        room: &RoomId,
        name: &str,
        content: &str,
    ) -> Result<DocId, SwitchError> {
        let args = json!({
            "name": name,
            "description": name,
            "instructions": "Written by the notary; read only.",
            "content": content,
        });
        let v = self.room_op(room, "create_room_document", args).await?;
        v.get("document_id")
            .and_then(Value::as_str)
            .map(|id| DocId(id.to_owned()))
            .ok_or_else(|| SwitchError("create_room_document returned no document_id".into()))
    }

    async fn update_document(
        &self,
        room: &RoomId,
        doc: &DocId,
        content: &str,
    ) -> Result<(), SwitchError> {
        let args = json!({ "document_id": doc.0, "content": content });
        self.room_op(room, "update_room_document", args)
            .await
            .map(drop)
    }

    async fn request_signature(&self, card: &SignCard) -> Result<RequestId, SwitchError> {
        let card = sign_card(card);
        let view = self
            .client
            .open_approval(&self.session_id, &card)
            .await
            .map_err(err)?;
        Ok(RequestId(view.request_id))
    }

    async fn ack_outcome(&self, request: &RequestId) -> Result<(), SwitchError> {
        let session = self
            .outcome_sessions
            .lock()
            .expect("sessions lock")
            .remove(&request.0)
            .unwrap_or_else(|| self.session_id.clone());
        self.client
            .ack_approval(&session, &request.0)
            .await
            .map(drop)
            .map_err(err)
    }
}

/// Every field explicit: `sign-D-0007-buyer-3f9a1c2e` (a later card for the same side and
/// terms gets `-2`, `-3`, ...), turn `D-0007-r5`, the side and hash in the title.
pub(super) fn sign_card(card: &SignCard) -> ApprovalCard {
    let hash8 = card.terms_hash.short();
    let (deal, label) = (&card.deal, &card.label);
    let mut request_id = format!("sign-{deal}-{label}-{hash8}");
    if card.attempt > 1 {
        request_id.push_str(&format!("-{}", card.attempt));
    }
    ApprovalCard {
        request_id,
        turn_id: format!("{deal}-r{}", card.round),
        title: format!("{deal}: {label}, sign the agreed terms {hash8}"),
        detail: Some(format!(
            "Sign binds the {label} side to the terms with hash {}. Only the bound owner's answer counts.",
            card.terms_hash
        )),
        options: vec![
            ApprovalOption {
                id: SIGN_OPTION.into(),
                label: "Sign".into(),
                decision: Decision::Accept,
            },
            ApprovalOption {
                id: DECLINE_OPTION.into(),
                label: "Decline".into(),
                decision: Decision::Decline,
            },
        ],
        room_id: Some(card.room.as_str().to_owned()),
        thread_id: None,
        expires_at: Some(rfc3339::format(card.expires)),
    }
}

/// An answer with no `answered_by` cannot be attributed, so it closes the card like an expiry.
pub(super) fn outcome(v: &ApprovalView) -> Option<CardOutcome> {
    let by = v
        .answered_by
        .as_deref()
        .and_then(|b| ParticipantId::new(b).ok());
    let answer = match (v.state.as_str(), v.answer.as_deref(), by) {
        ("answered", Some(SIGN_OPTION), Some(by)) => CardAnswer::Sign(by),
        ("answered", Some(_), Some(by)) => CardAnswer::Decline(by),
        ("answered" | "expired" | "closed", ..) => CardAnswer::Expired,
        _ => return None,
    };
    Some(CardOutcome {
        request: RequestId(v.request_id.clone()),
        answer,
    })
}

impl SignatureOutcomes for SwitchAdapter {
    async fn undelivered(&self) -> Result<Vec<CardOutcome>, SwitchError> {
        let views = self.client.approval_outcomes().await.map_err(err)?;
        let mut sessions = self.outcome_sessions.lock().expect("sessions lock");
        Ok(views
            .iter()
            .filter(|v| v.kind == "approval")
            .filter_map(|v| {
                let o = outcome(v)?;
                sessions.insert(v.request_id.clone(), v.session_id.clone());
                Some(o)
            })
            .collect())
    }
}

impl RosterPort for SwitchAdapter {
    async fn agent_names(&self, room: &RoomId) -> Result<Vec<String>, SwitchError> {
        let people = self.client.participants(room.as_str()).await.map_err(err)?;
        Ok(people
            .into_iter()
            .filter(|p| p.kind == "agent")
            .map(|p| p.name)
            .collect())
    }
}
