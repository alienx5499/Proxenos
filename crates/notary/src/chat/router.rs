//! Maps (sender, room, command) to a use case; dedupes on message_id; picks the reply room.
//!
//! Every reply is a root post through `SwitchPort::post`, never a thread reply. Command
//! handlers live in `kickoff`, `parties`, `moves` and `cards`.

use std::sync::Mutex;

use proxenos_app::AppError;
use proxenos_app::ports::{Clock, DealReader, RepoError, SwitchPort};
use proxenos_app::stored::StoredDeal;
use proxenos_domain::date::Date;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::party::{ParticipantId, Role, RoomId};

use super::canned::Canned;
use super::desk_replies::ErrorWakes;
use super::grammar::{Command, ParseError};
use super::inbound::{Inbound, SenderKind};
use super::kickoff::PendingDesk;
use super::rate::RateLimit;
use super::{render, replies};

pub use super::router_ports::{Dropped, Ports, Routed, RouterConfig, SeenMessages};

/// Tries per command when the deal changed under it (an HTTP or fixture write in between).
const CONFLICT_ATTEMPTS: u32 = 3;

/// Where a command was sent, relative to the deal that owns the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    DealRoom,
    Desk(Role),
}

pub(super) struct Ctx {
    pub stored: StoredDeal,
    pub place: Place,
    pub room: RoomId,
    pub sender: ParticipantId,
}

impl Ctx {
    pub fn id(&self) -> &DealId {
        self.stored.deal.id()
    }
}

/// One router per notary process, driven by a single task: every inbound message, card poll
/// and wake goes through that one task, in order. It is `!Sync` by contract (the type does
/// not enforce it): concurrent calls would race on the optimistic store version and on the
/// pending-desk list. Posts are best-effort: a Switch failure after a committed move is
/// logged and never turns the result into an error.
pub struct Router<P: Ports> {
    pub(super) p: P,
    pub(super) cfg: RouterConfig,
    /// Desk kickoffs that arrived before their deal's open kickoff.
    pub(super) pending_desks: Mutex<Vec<PendingDesk>>,
    rate: RateLimit,
    pub(super) error_wakes: ErrorWakes,
}

impl<P: Ports> Router<P> {
    pub fn new(ports: P, config: RouterConfig) -> Self {
        Self {
            p: ports,
            cfg: config,
            pending_desks: Mutex::default(),
            rate: RateLimit::default(),
            error_wakes: ErrorWakes::default(),
        }
    }

    pub fn ports(&self) -> &P {
        &self.p
    }

    /// Port and store failures are returned, after a visible "not recorded" line in the
    /// room; every refusal is a reply. A version conflict reloads the deal and re-runs the
    /// command. The message id is marked seen first, so a command runs at most once.
    pub async fn handle(&self, msg: Inbound) -> Result<Routed, AppError> {
        if let Some(d) = self.drop_reason(&msg).await? {
            return Ok(Routed::Dropped(d));
        }
        let mut attempt = 1;
        loop {
            match self.route(&msg).await {
                Err(AppError::Repo(RepoError::Conflict)) if attempt < CONFLICT_ATTEMPTS => {
                    attempt += 1;
                }
                Err(e) => {
                    self.post(&msg.room, &replies::not_recorded()).await?;
                    return Err(e);
                }
                done => return done,
            }
        }
    }

    async fn route(&self, msg: &Inbound) -> Result<Routed, AppError> {
        let today = Date::containing(self.p.clock().now());
        let parsed = msg.command().and_then(|cmd| match &cmd {
            Command::Probe { terms } | Command::Offer { terms, .. }
                if terms.has_date_before(today) =>
            {
                Err(ParseError::PastDate)
            }
            _ => Ok(cmd),
        });
        if msg.sender_kind == SenderKind::Platform {
            return match parsed {
                Ok(Command::Open {
                    kind,
                    deal_id,
                    opener,
                }) => self.open(msg, kind, deal_id, opener).await,
                Ok(Command::Desk { side, deal_id }) => self.desk(msg, side, deal_id).await,
                _ => Ok(Routed::Dropped(Dropped::Platform)),
            };
        }
        let cmd = match parsed {
            Ok(Command::Open { .. } | Command::Desk { .. }) => {
                return Ok(Routed::Dropped(Dropped::KickoffNotFromOperator));
            }
            Ok(Command::Join { .. }) if msg.sender_kind == SenderKind::Agent => {
                let text = replies::desk_message("AgentCannotJoin", "join is for owners");
                return self.post(&msg.room, &text).await;
            }
            Ok(Command::Join { code }) => return self.join(msg, code).await,
            Ok(cmd) => cmd,
            Err(e) => {
                let Some(ctx) = self.context(&msg.room, &msg.sender).await? else {
                    return Ok(Routed::Dropped(Dropped::NoDeal));
                };
                let deal = &ctx.stored.deal;
                if msg.sender_kind == SenderKind::Agent
                    && deal.owner_role(&msg.sender).is_none()
                    && deal.negotiator_role(&msg.sender).is_none()
                {
                    return Ok(Routed::Dropped(Dropped::UnboundAgentText));
                }
                let text = match (ctx.place, e) {
                    (Place::Desk(_), e) => {
                        return self.desk_refusal(&ctx, render::parse_error(&e)).await;
                    }
                    (Place::DealRoom, ParseError::PastDate) => ParseError::PastDate.to_string(),
                    (Place::DealRoom, _) => "command not understood".into(),
                };
                return self.post(&msg.room, &text).await;
            }
        };
        let Some(ctx) = self.context(&msg.room, &msg.sender).await? else {
            return Ok(Routed::Dropped(Dropped::NoDeal));
        };
        self.dispatch(msg, ctx, cmd).await
    }

    async fn drop_reason(&self, msg: &Inbound) -> Result<Option<Dropped>, AppError> {
        let now = self.p.clock().now();
        if !self.p.seen().mark_seen(&msg.message_id, now).await? {
            return Ok(Some(Dropped::Duplicate));
        }
        Ok(if !msg.addressed {
            Some(Dropped::Unaddressed)
        } else if msg.sender == self.cfg.notary {
            Some(Dropped::OwnMessage)
        } else if let Some(c) = msg.canned() {
            Some(match c {
                Canned::Refusal => Dropped::Refusal,
                Canned::AutoReply => Dropped::AutoReply,
            })
        } else if msg.sender_kind == SenderKind::Platform
            && msg.on_behalf_of.as_deref() != Some(self.cfg.operator.as_str())
        {
            Some(Dropped::Platform)
        } else if msg.sender_kind != SenderKind::Platform && !self.rate.allow(&msg.sender, now) {
            Some(Dropped::RateLimited)
        } else {
            None
        })
    }

    /// The live deal using `room`, else the most recent closed one.
    pub(super) async fn context(
        &self,
        room: &RoomId,
        sender: &ParticipantId,
    ) -> Result<Option<Ctx>, AppError> {
        let mut deals = self.p.repo().in_room(room).await?;
        deals.sort_by(|a, b| a.deal.id().cmp(b.deal.id()));
        let pick = deals
            .iter()
            .position(|s| !s.deal.state().is_terminal())
            .or(deals.len().checked_sub(1));
        let Some(stored) = pick.map(|i| deals.swap_remove(i)) else {
            return Ok(None);
        };
        let place = [Role::A, Role::B]
            .into_iter()
            .find(|r| stored.deal.desk(*r) == Some(room))
            .map_or(Place::DealRoom, Place::Desk);
        Ok(Some(Ctx {
            stored,
            place,
            room: room.clone(),
            sender: sender.clone(),
        }))
    }

    async fn dispatch(&self, msg: &Inbound, ctx: Ctx, cmd: Command) -> Result<Routed, AppError> {
        if cmd == Command::Status && ctx.place == Place::DealRoom {
            return self
                .post(&ctx.room, &render::status_line(&ctx.stored.deal))
                .await;
        }
        let deal = &ctx.stored.deal;
        let owner = deal.owner_role(&msg.sender);
        let negotiator = deal.negotiator_role(&msg.sender);
        match (cmd, owner, negotiator) {
            (cmd, None, None) => self.unbound(msg, &ctx, &cmd).await,
            (Command::Sign { hash8 }, Some(_), _) => self.sign(msg, &ctx, hash8).await,
            (Command::Status, Some(r), _) if ctx.place == Place::Desk(r) => {
                self.desk_status(&ctx, r).await
            }
            (Command::Withdraw, Some(r), _) => self.withdraw(msg, &ctx, r).await,
            (Command::State, _, Some(r)) if ctx.place == Place::Desk(r) => {
                self.state(&ctx, r).await
            }
            (Command::Kinds, _, Some(r)) if ctx.place == Place::Desk(r) => {
                self.kinds(&ctx, r).await
            }
            (Command::Probe { terms }, _, Some(r)) => self.probe(msg, &ctx, r, terms).await,
            (Command::Offer { terms, note }, _, Some(r)) => {
                self.offer(msg, &ctx, r, terms, note).await
            }
            (Command::Accept { offer_id }, _, Some(r)) => self.accept(msg, &ctx, r, offer_id).await,
            (Command::Sign { .. } | Command::Withdraw, None, Some(_)) => {
                self.refuse(&ctx, DomainError::NotBoundOwner.into()).await
            }
            (_, Some(_), None) => {
                self.refuse(&ctx, DomainError::NotBoundNegotiator.into())
                    .await
            }
            _ => self.refuse(&ctx, DomainError::WrongRoom.into()).await,
        }
    }

    /// The one unbound sender allowed past this point: the negotiator's first `state` or
    /// `kinds` in a desk room whose owner is bound and whose negotiator is not.
    async fn unbound(&self, msg: &Inbound, ctx: &Ctx, cmd: &Command) -> Result<Routed, AppError> {
        if matches!(cmd, Command::State | Command::Kinds)
            && let Place::Desk(role) = ctx.place
            && let Some(binding) = ctx.stored.deal.binding(role)
            && binding.agent.is_none()
            && !self.cfg.relays.contains(&msg.sender)
        {
            let owner = binding.owner.clone();
            return self.bind_negotiator(msg, ctx, role, owner).await;
        }
        let text = render::not_a_party(ctx.id());
        match ctx.place {
            Place::DealRoom => self.post(&ctx.room, &text).await,
            Place::Desk(_) => {
                self.post(&ctx.room, &replies::desk_message("NotAParty", &text))
                    .await
            }
        }
    }

    /// Best-effort: a failed post is logged, never returned.
    pub(super) async fn post(&self, room: &RoomId, text: &str) -> Result<Routed, AppError> {
        if let Err(e) = self.p.switch().post(room, text).await {
            eprintln!("post to {room} failed: {e}");
        }
        Ok(Routed::Handled)
    }

    /// A refusal in the room the command came from: a desk gets the error block (see
    /// `desk_refusal`), the deal room only the message. Store and port failures are returned.
    pub(super) async fn refuse(&self, ctx: &Ctx, err: AppError) -> Result<Routed, AppError> {
        match ctx.place {
            Place::Desk(_) => {
                let block = replies::desk_app_error(&err).ok_or(err)?;
                self.desk_refusal(ctx, block).await
            }
            Place::DealRoom => {
                let line = replies::refusal_line(&err).ok_or(err)?;
                self.post(&ctx.room, &line).await
            }
        }
    }
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
