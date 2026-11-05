//! Addressed event in, `claude -p` / `codex exec`, one command out.

use futures_util::StreamExt;
use proxenos_switch_client::SwitchClient;
use proxenos_switch_client::stream::{EventKind, Item, Message, StreamSpec};

use crate::BenchError;
use crate::cli::AgentCli;
use crate::extract::{Grammar, Reply, extract};
use crate::prereg::Side;
use crate::prompt;

/// One side's negotiator: its Switch identity, its CLI and its standing instructions.
pub struct Seat<C> {
    pub side: Side,
    pub client: SwitchClient,
    pub cli: C,
    pub grammar: Grammar,
    pub standing: String,
    /// Who commands go to: `notary`, or the counterparty agent's name in arm D.
    pub address: String,
    pub deal_room: String,
    /// `--play` acts only in the deal room and, when known, this desk room.
    pub desk_room: Option<String>,
    /// Participant id of the notary; state blocks are read only from its messages.
    pub notary_sender: Option<String>,
    pub last_state: Option<String>,
    /// The last post, cleared by the next notary event; the same text is never sent twice in a row.
    pub last_sent: Option<String>,
    /// This round's commands and notary replies: each CLI turn starts with no memory.
    pub round_log: RoundLog,
}

/// Lines kept per round; enough for 3 probes, an offer and their replies.
const ROUND_LOG_LINES: usize = 12;

/// What one seat sent and was told in the current round, state blocks left out.
#[derive(Default)]
pub struct RoundLog {
    round: Option<u64>,
    lines: Vec<String>,
}

impl RoundLog {
    /// Starts afresh when `state` names a round other than the logged one.
    fn enter(&mut self, state: Option<&str>) {
        let round = state
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v["round"].as_u64());
        if round.is_some() && round != self.round {
            (self.round, self.lines) = (round, Vec::new());
        }
    }

    fn push(&mut self, who: &str, text: &str) {
        let kept: Vec<&str> = text
            .split("```json")
            .filter(|part| !part.contains("\"my_role\""))
            .collect();
        let line: String = kept
            .join("```json")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.lines.push(format!(
            "{who}: {}",
            line.chars().take(400).collect::<String>()
        ));
        let over = self.lines.len().saturating_sub(ROUND_LOG_LINES);
        self.lines.drain(..over);
    }
}

/// What one CLI turn produced.
#[derive(Clone, Debug)]
pub struct Turn {
    pub raw: String,
    pub error: Option<String>,
    pub reply: Reply,
}

impl<C: AgentCli> Seat<C> {
    /// Whether an addressed message is worth a CLI turn; keeps any notary state block.
    /// From the notary only a turn wake (`my_turn` true), a refusal (an `error` field) or the
    /// bind request count; acknowledgements just refresh the state. Anyone else always counts.
    pub fn wants_turn(&mut self, sender: &str, body: &str) -> bool {
        match self.notary_sender.as_deref() {
            Some(n) if n == sender => {}
            // Without a notary every event is new; with one, others don't reset the guard.
            None => {
                self.last_sent = None;
                return true;
            }
            Some(_) => return true,
        }
        self.last_sent = None;
        let state = prompt::state_block(body);
        let my_turn = state
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .is_some_and(|v| v["my_turn"] == true);
        if state.is_some() {
            self.last_state = state;
        }
        my_turn
            || prompt::json_objects(body).any(|v| v.get("error").is_some())
            || body.contains("send state to bind")
    }

    /// The post for a turn, or `None` when it repeats the last post with no notary event since.
    pub fn post_for(&mut self, turn: &Turn) -> Option<String> {
        let content = outgoing(&self.address, turn)?;
        if self.last_sent.as_deref() == Some(content.as_str()) {
            eprintln!("[{}] suppressed a repeat of: {content}", self.side);
            return None;
        }
        self.last_sent = Some(content.clone());
        self.round_log.push("you", &content);
        Some(content)
    }

    /// Runs the CLI on one addressed message. A CLI failure is a turn with no reply.
    /// `state` overrides the last state block seen (arm D passes the driver's ledger).
    pub async fn think(
        &mut self,
        room: &str,
        sender: &str,
        body: &str,
        state: Option<String>,
    ) -> Turn {
        if self.notary_sender.as_deref() == Some(sender)
            && let Some(s) = prompt::state_block(body)
        {
            self.last_state = Some(s);
        }
        let label = if room == self.deal_room {
            "deal room"
        } else {
            "desk room"
        };
        let state = state.or_else(|| self.last_state.clone());
        self.round_log.enter(state.as_deref());
        let text = prompt::turn(
            &self.standing,
            &self.round_log.lines,
            label,
            body,
            state.as_deref(),
        );
        let who = if self.notary_sender.as_deref() == Some(sender) {
            "notary"
        } else {
            sender
        };
        self.round_log.push(who, body);
        let (raw, error) = match self.cli.run(&text).await {
            Ok(raw) => (raw, None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        let reply = extract(&raw, self.grammar);
        Turn { raw, error, reply }
    }
}

/// The post a turn makes: a command addressed to `address`, or the reply as text.
/// Every `@` the agent wrote is dropped, so only the leading mention added here addresses anyone.
pub fn outgoing(address: &str, turn: &Turn) -> Option<String> {
    match &turn.reply {
        Reply::Command { text, .. } => Some(format!("@{address} {}", text.replace('@', ""))),
        Reply::Text(t) => Some(t.replace('@', "")).filter(|t| !t.trim().is_empty()),
        Reply::Empty => None,
    }
}

/// The room and message of an addressed message event; everything else is `None`.
pub fn addressed(item: Item) -> Option<(String, Message)> {
    match item {
        Item::Event(e) => match e.kind {
            EventKind::Message(m) if m.addressed => Some((e.room_id, m)),
            _ => None,
        },
        Item::Gap(g) => {
            eprintln!("stream gap ({}); continuing", g.reason);
            None
        }
        Item::SubscriptionChanged(c) => {
            eprintln!(
                "warning: subscription changed ({}), rooms {:?}",
                c.reason, c.rooms
            );
            None
        }
        Item::Connected(_) => None,
    }
}

/// `--play`: answers addressed messages that warrant a turn, live, until the turn budget is spent.
pub async fn play<C: AgentCli>(seat: &mut Seat<C>, budget: u32) -> Result<(), BenchError> {
    let mut sub = seat.client.subscribe(StreamSpec::all_addressed());
    let mut turns = 0;
    while turns < budget {
        let Some(item) = sub.next().await else {
            return Err(BenchError::Input("event stream ended".into()));
        };
        let Some((room, msg)) = item.map_or_else(
            |e| {
                eprintln!("stream: {e}");
                None
            },
            addressed,
        ) else {
            continue;
        };
        if room != seat.deal_room && seat.desk_room.as_deref() != Some(room.as_str()) {
            eprintln!("ignored an addressed message in room {room} (not this deal)");
            continue;
        }
        if !seat.wants_turn(&msg.sender, &msg.body) {
            continue;
        }
        let turn = seat.think(&room, &msg.sender, &msg.body, None).await;
        turns += 1;
        if let Some(e) = &turn.error {
            eprintln!("turn {turns}: {e}");
        }
        if let Some(content) = seat.post_for(&turn) {
            seat.client.post_message(&room, &content).await?;
            eprintln!("turn {turns} [{}] {content}", seat.side);
        }
    }
    eprintln!("turn budget of {budget} spent");
    Ok(())
}

#[cfg(test)]
#[path = "agent_loop_tests.rs"]
pub mod tests;
