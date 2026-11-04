//! Outbound calls that need no room claim: /message, roster, generic ops, aliases, media upload.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Error, QUICK_TIMEOUT, SwitchClient, UPLOAD_TIMEOUT, enc};

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Participant {
    /// `agent` or `user`.
    #[serde(rename = "type")]
    pub kind: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub status: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Upload {
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
struct Posted {
    event_id: Option<String>,
}

#[derive(Deserialize)]
struct Roster {
    participants: Vec<Participant>,
}

#[derive(Deserialize)]
struct OpResult {
    #[serde(default)]
    result: Value,
}

impl SwitchClient {
    /// Posts as the agent with no connection. A leading `@name` addresses that participant.
    /// Returns the event id when the server reports one.
    pub async fn post_message(
        &self,
        room_id: &str,
        content: &str,
    ) -> Result<Option<String>, Error> {
        let body = json!({ "room_id": room_id, "content": content });
        let req = self.http.post(self.agent_url("/message")).json(&body);
        let p: Posted = self.call_within(req, QUICK_TIMEOUT).await?;
        Ok(p.event_id)
    }

    pub async fn participants(&self, room_id: &str) -> Result<Vec<Participant>, Error> {
        let url = self.url(&format!("/agents/rooms/{}/participants", enc(room_id)));
        let roster: Roster = self.call_within(self.http.get(url), QUICK_TIMEOUT).await?;
        Ok(roster.participants)
    }

    /// `POST /agents/{id}/ops/{name}`; `args` is the arguments object itself.
    /// `connection_id` binds the call to that connection's room.
    pub async fn op(
        &self,
        name: &str,
        args: &Value,
        connection_id: Option<&str>,
    ) -> Result<Value, Error> {
        let mut req = self
            .http
            .post(self.agent_url(&format!("/ops/{}", enc(name))))
            .json(args);
        if let Some(c) = connection_id {
            req = req.header("X-Switch-Connection-Id", c);
        }
        Ok(self.call::<OpResult>(req).await?.result)
    }

    /// Sets per-room aliases (agent name to alias; "" clears). Needs no connection.
    pub async fn update_room_aliases(
        &self,
        room_id: &str,
        aliases: &BTreeMap<String, String>,
    ) -> Result<Value, Error> {
        self.op(
            "update_room",
            &json!({ "room_id": room_id, "aliases": aliases }),
            None,
        )
        .await
    }

    /// Posts files as one logical message, with an optional caption.
    pub async fn upload_media(
        &self,
        room_id: &str,
        files: Vec<Upload>,
        caption: Option<&str>,
    ) -> Result<Value, Error> {
        let mut form = reqwest::multipart::Form::new();
        for f in files {
            let part = reqwest::multipart::Part::bytes(f.bytes)
                .file_name(f.filename)
                .mime_str(&f.mime)?;
            form = form.part("files", part);
        }
        if let Some(c) = caption {
            form = form.text("caption", c.to_owned());
        }
        let url = self.agent_url(&format!("/rooms/{}/media", enc(room_id)));
        self.call_within(self.http.post(url).multipart(form), UPLOAD_TIMEOUT)
            .await
    }
}
