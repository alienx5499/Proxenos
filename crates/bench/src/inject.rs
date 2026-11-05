//! The scenario's injection script: the human's addressed message and the scripted note.

use serde_json::Value;

use crate::BenchError;
use crate::agent_loop::Turn;
use crate::cli::AgentCli;
use crate::extract::Reply;
use crate::prereg::Side;
use crate::record::Posted;
use crate::run::{Progress, Run};

impl<C: AgentCli> Run<C> {
    /// The counterparty's first offer from the injection round on carries the scripted note.
    pub(crate) fn inject_note(&self, p: &mut Progress, side: Side, turn: &mut Turn) -> bool {
        let Some(inj) = &self.injection else {
            return false;
        };
        if p.rec.injection.note_injected || side != inj.target.other() || p.round < inj.at_round {
            return false;
        }
        let Reply::Command { text, .. } = &mut turn.reply else {
            return false;
        };
        let Some(mut v) = text
            .strip_prefix("offer ")
            .and_then(|a| serde_json::from_str::<Value>(a).ok())
        else {
            return false;
        };
        v["note"] = Value::String(inj.content_note.clone());
        *text = format!("offer {v}");
        p.rec.injection.note_injected = true;
        true
    }

    pub(crate) async fn inject_human(&mut self, p: &mut Progress) -> Result<(), BenchError> {
        let Some(inj) = &self.injection else {
            return Ok(());
        };
        if p.rec.injection.human_posted || p.round < inj.at_round {
            return Ok(());
        }
        let injector = self.injector.as_ref().ok_or_else(|| {
            BenchError::Input(
                "scenario has an injection but no injector agent is configured".into(),
            )
        })?;
        let name = &self.names[inj.target as usize];
        let content = format!("@{name} {}", inj.human_text);
        let room = self.buyer.deal_room.clone();
        injector.post_message(&room, &content).await?;
        p.rec.injection.human_posted = true;
        p.rec.posted.push(Posted {
            side: None,
            by_driver: false,
            room,
            content,
            raw: String::new(),
            cli_error: None,
            note_injected: false,
        });
        Ok(())
    }
}
