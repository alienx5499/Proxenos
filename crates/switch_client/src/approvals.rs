//! Sign cards via /agent-sessions/{id}/approvals. Conditional: built only if L16 passes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, SwitchClient, enc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ApprovalOption {
    pub id: String,
    pub label: String,
    pub decision: Decision,
}

/// An `approval` card. Every field is sent, nulls included: the server requires them all.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ApprovalCard {
    pub request_id: String,
    pub turn_id: String,
    pub title: String,
    pub detail: Option<String>,
    pub options: Vec<ApprovalOption>,
    pub room_id: Option<String>,
    /// Keep `None`: a card in a thread is hidden on Mattermost.
    pub thread_id: Option<String>,
    /// RFC 3339.
    pub expires_at: Option<String>,
}

#[derive(Serialize)]
struct Open<'a> {
    #[serde(flatten)]
    card: &'a ApprovalCard,
    kind: &'static str,
    questions: [Value; 0],
}

/// A request's state as the server reports it (camelCase on the wire).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub session_id: String,
    pub request_id: String,
    pub kind: String,
    /// `open`, `answered`, `expired` or `closed`.
    pub state: String,
    /// The chosen option id.
    pub answer: Option<String>,
    pub answers: Option<Vec<Value>>,
    /// The answering person's platform participant id.
    pub answered_by: Option<String>,
    pub answered_at: Option<String>,
    pub expires_at: Option<String>,
    pub delivered_at: Option<String>,
}

impl SwitchClient {
    pub async fn open_approval(
        &self,
        session_id: &str,
        card: &ApprovalCard,
    ) -> Result<ApprovalView, Error> {
        let body = Open {
            card,
            kind: "approval",
            questions: [],
        };
        self.post_json(
            self.url(&format!("/agent-sessions/{}/approvals", enc(session_id))),
            &body,
        )
        .await
    }

    /// Answers and expiries not yet acknowledged, oldest first. Poll it.
    pub async fn approval_outcomes(&self) -> Result<Vec<ApprovalView>, Error> {
        self.get_json(self.url("/agent-sessions/approvals/outcomes"))
            .await
    }

    pub async fn ack_approval(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<ApprovalView, Error> {
        let url = self.url(&format!(
            "/agent-sessions/{}/approvals/{}/delivered",
            enc(session_id),
            enc(request_id)
        ));
        self.call(self.http.post(url)).await
    }
}
