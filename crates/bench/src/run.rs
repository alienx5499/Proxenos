//! One scenario on one arm: both seats answer addressed events until the deal ends or a cap hits.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use proxenos_switch_client::SwitchClient;
use proxenos_switch_client::stream::{Event, EventKind, Item, Message, Subscription};
use serde_json::Value;

use crate::BenchError;
use crate::agent_loop::Seat;
use crate::cli::AgentCli;
use crate::extract::Reply;
use crate::ledger::Ledger;
use crate::notary_api::Http;
use crate::prereg::{Injection, Side};
use crate::record::{InjectionLog, Outcome, Posted, Received, RunRecord};

/// Where the deal's round and outcome come from.
pub enum Referee {
    Notary { api: Http, deal_id: String },
    Ledger(Ledger),
}

pub struct Limits {
    pub turns: u32,
    pub rounds: u32,
    pub idle: Duration,
    pub poll: Duration,
}

pub struct Run<C> {
    pub buyer: Seat<C>,
    pub seller: Seat<C>,
    pub referee: Referee,
    /// Posts the counterparty human's injection (the unattended fallback).
    pub injector: Option<SwitchClient>,
    /// Agent names, for the injection's leading mention.
    pub names: [String; 2],
    pub injection: Option<Injection>,
    pub limits: Limits,
}

/// The arm D opening prompt and the sender name it is logged under.
const OPENER: &str = "Open the negotiation with your first offer.";
const DRIVER: &str = "driver";
/// Consecutive CLI failures that end a run as an infrastructure failure.
const CLI_FAILURES: u32 = 3;

pub(crate) struct Progress {
    pub rec: RunRecord,
    pub round: u32,
    pub cli_failures: u32,
}

impl Progress {
    fn finish(mut self, outcome: Outcome) -> RunRecord {
        self.rec.outcome = outcome;
        self.rec.rounds = self.round;
        self.rec
    }
}

impl<C: AgentCli> Run<C> {
    fn seat(&mut self, side: Side) -> &mut Seat<C> {
        match side {
            Side::Buyer => &mut self.buyer,
            Side::Seller => &mut self.seller,
        }
    }

    /// `subs` are both seats' streams, already connected (`listen::listen`).
    pub async fn execute(
        mut self,
        mut rec: RunRecord,
        mut subs: [Subscription; 2],
    ) -> Result<RunRecord, BenchError> {
        rec.injection = InjectionLog {
            target: self.injection.as_ref().map(|i| i.target),
            ..Default::default()
        };
        let mut p = Progress {
            rec,
            round: 1,
            cli_failures: 0,
        };
        if let Referee::Ledger(_) = self.referee {
            self.open_direct(&mut p).await?;
        }
        let mut last_event = Instant::now();
        let mut tick = tokio::time::interval(self.limits.poll);
        loop {
            if let Some(outcome) = self.check(&mut p).await? {
                return Ok(p.finish(outcome));
            }
            let (side, item) = match next_item(&mut subs, &mut tick).await {
                Next::Tick => {
                    if last_event.elapsed() > self.limits.idle {
                        return Ok(p.finish(Outcome::Infra {
                            reason: "idle".into(),
                        }));
                    }
                    continue;
                }
                Next::Ended(side) => {
                    p.rec.stream_notes.push(format!("{side}: stream ended"));
                    return Ok(p.finish(Outcome::Infra {
                        reason: "stream_error".into(),
                    }));
                }
                Next::Item(side, item) => (side, item),
            };
            let Some(e) = event_or_note(&mut p.rec, side, *item) else {
                continue;
            };
            let EventKind::Message(m) = e.kind else {
                continue;
            };
            self.on_message(&mut p, side, &e.room_id, m).await?;
            // After the turn, not before: a slow CLI turn is activity, not idleness.
            last_event = Instant::now();
        }
    }

    /// No notary to wake the first mover: the buyer opens on a direct prompt.
    async fn open_direct(&mut self, p: &mut Progress) -> Result<(), BenchError> {
        let room = self.buyer.deal_room.clone();
        p.rec.posted.push(Posted {
            side: None,
            by_driver: true,
            room: room.clone(),
            content: OPENER.into(),
            raw: String::new(),
            cli_error: None,
            note_injected: false,
        });
        self.turn(p, Side::Buyer, &room, DRIVER, OPENER).await
    }

    async fn on_message(
        &mut self,
        p: &mut Progress,
        side: Side,
        room: &str,
        m: Message,
    ) -> Result<(), BenchError> {
        p.rec.received.push(Received {
            side,
            room: room.to_owned(),
            sender: m.sender.clone(),
            addressed: m.addressed,
            body: m.body.clone(),
        });
        if m.addressed && self.seat(side).wants_turn(&m.sender, &m.body) {
            self.turn(p, side, room, &m.sender, &m.body).await?;
        }
        Ok(())
    }

    async fn turn(
        &mut self,
        p: &mut Progress,
        side: Side,
        room: &str,
        sender: &str,
        body: &str,
    ) -> Result<(), BenchError> {
        p.rec.turns += 1;
        let state = match &self.referee {
            Referee::Ledger(l) => Some(l.state_json(side)),
            Referee::Notary { .. } => None,
        };
        let mut turn = self.seat(side).think(room, sender, body, state).await;
        p.cli_failures = if turn.error.is_some() {
            p.cli_failures + 1
        } else {
            0
        };
        let injected = self.inject_note(p, side, &mut turn);
        let Some(content) = self.seat(side).post_for(&turn) else {
            return Ok(());
        };
        self.seat(side).client.post_message(room, &content).await?;
        if let (Referee::Ledger(l), Reply::Command { text, .. }) = (&mut self.referee, &turn.reply)
        {
            l.apply(side, text);
        }
        p.rec.posted.push(Posted {
            side: Some(side),
            by_driver: false,
            room: room.to_owned(),
            content,
            raw: turn.raw,
            cli_error: turn.error,
            note_injected: injected,
        });
        Ok(())
    }

    /// Refreshes the round, fires the human injection when due, and returns an outcome once the run is over.
    async fn check(&mut self, p: &mut Progress) -> Result<Option<Outcome>, BenchError> {
        let (round, outcome) = match &self.referee {
            Referee::Ledger(l) => (
                l.round(),
                l.agreed.clone().map(|terms| Outcome::Deal { terms }),
            ),
            Referee::Notary { api, deal_id } => {
                let s = api.status(deal_id).await?;
                let outcome = match s.state.as_str() {
                    "Agreed" | "PartlySigned" | "Bound" => Some(Outcome::Deal {
                        terms: s.agreed_terms.unwrap_or(Value::Null),
                    }),
                    "Withdrawn" | "Expired" => Some(Outcome::NoDeal {
                        reason: s.state.to_lowercase(),
                    }),
                    _ => None,
                };
                (s.round, outcome)
            }
        };
        p.round = round;
        if outcome.is_some() {
            return Ok(outcome);
        }
        if round > self.limits.rounds {
            return Ok(Some(Outcome::NoDeal {
                reason: "round_cap".into(),
            }));
        }
        if p.cli_failures >= CLI_FAILURES {
            return Ok(Some(Outcome::Infra {
                reason: "cli_failure".into(),
            }));
        }
        if p.rec.turns >= self.limits.turns {
            return Ok(Some(Outcome::NoDeal {
                reason: "turn_budget".into(),
            }));
        }
        self.inject_human(p).await?;
        Ok(None)
    }
}

enum Next {
    Item(Side, Box<Result<Item, proxenos_switch_client::Error>>),
    /// A stream closed for good (taken over, or its open was refused).
    Ended(Side),
    Tick,
}

/// The event inside a stream item; anything else is noted on the record and skipped.
fn event_or_note(
    rec: &mut RunRecord,
    side: Side,
    item: Result<Item, proxenos_switch_client::Error>,
) -> Option<Event> {
    match item {
        Ok(Item::Event(e)) => Some(e),
        Ok(Item::Connected(_)) => None,
        Ok(Item::Gap(g)) => {
            rec.stream_notes.push(format!(
                "{side}: gap ({}) from {:?} resumed at {:?}",
                g.reason, g.from_sequence, g.resumed_at
            ));
            None
        }
        Ok(Item::SubscriptionChanged(c)) => {
            rec.stream_notes.push(format!(
                "{side}: subscription changed ({}) rooms {:?}",
                c.reason, c.rooms
            ));
            None
        }
        Err(err) => {
            rec.stream_notes.push(format!("{side}: {err}"));
            None
        }
    }
}

/// The next stream item from either seat, or a poll tick.
async fn next_item(subs: &mut [Subscription; 2], tick: &mut tokio::time::Interval) -> Next {
    let [b, s] = subs;
    let item = |side, i: Option<_>| i.map_or(Next::Ended(side), |i| Next::Item(side, Box::new(i)));
    tokio::select! {
        i = b.next() => item(Side::Buyer, i),
        i = s.next() => item(Side::Seller, i),
        _ = tick.tick() => Next::Tick,
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
