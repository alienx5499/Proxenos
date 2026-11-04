//! Binding and owner commands: join, the negotiator's first command, sign, withdraw, status,
//! state, and the turn wake.

use proxenos_app::AppError;
use proxenos_app::commands::bind_negotiator::{BindNegotiator, bind_negotiator_named};
use proxenos_app::commands::bind_party::{BindParty, bind_party};
use proxenos_app::commands::record_signature::{RecordSignature, record_signature};
use proxenos_app::commands::withdraw::{Withdraw, withdraw};
use proxenos_app::ports::{Clock, DealReader, RosterPort};
use proxenos_app::queries::get_party_view::get_party_view;
use proxenos_app::queries::get_status::get_status;
use proxenos_domain::date::Date;
use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::limit::OwnVerdict;
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::signature::MessageId;

use super::inbound::Inbound;
use super::render;
use super::router::{Ctx, Place, Ports, Routed, Router};

impl<P: Ports> Router<P> {
    pub(super) async fn join(&self, msg: &Inbound, code: String) -> Result<Routed, AppError> {
        if let Some(ctx) = self.context(&msg.room, &msg.sender).await?
            && ctx.place == Place::DealRoom
        {
            return self.post(&msg.room, "join only in your desk room").await;
        }
        let p = &self.p;
        let cmd = BindParty {
            join_code: code,
            sender: msg.sender.clone(),
            room: msg.room.clone(),
        };
        let (id, role) = match bind_party(p.repo(), p.clock(), p.secrets(), cmd).await {
            Ok(bound) => bound,
            Err(e) => {
                let text = super::replies::desk_app_error(&e).ok_or(e)?;
                return self.post(&msg.room, &text).await;
            }
        };
        let deal = self.load(&id).await?;
        let kind = deal.deal.kind();
        self.post(&msg.room, &render::bound(&id, kind, role))
            .await?;
        match self.desk_agents(&msg.room).await {
            Ok(names) => {
                for name in names {
                    self.post(&msg.room, &render::wake_to_bind(&name)).await?;
                }
            }
            Err(e) => eprintln!("roster of {} unavailable, no bind wake: {e}", msg.room),
        }
        self.post(deal.deal.room(), &render::joined(kind, role))
            .await
    }

    pub(super) async fn bind_negotiator(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        role: Role,
        owner: ParticipantId,
    ) -> Result<Routed, AppError> {
        let mut excluded = vec![self.cfg.notary.clone(), owner];
        excluded.extend(self.cfg.relays.iter().cloned());
        excluded.extend(ParticipantId::new(self.cfg.operator.as_str()).ok());
        let name = self.negotiator_name(msg, &ctx.room).await?;
        let cmd = BindNegotiator {
            deal: ctx.id().clone(),
            sender: msg.sender.clone(),
            room: ctx.room.clone(),
            excluded,
        };
        let bound = bind_negotiator_named(self.p.repo(), self.p.clock(), cmd, name).await;
        if let Err(e) = bound {
            return self.refuse(ctx, e).await;
        }
        let view = get_party_view(self.p.repo(), ctx.id(), role).await?;
        if view.my_turn {
            // The turn wake carries the same state block: one addressed message, not two.
            self.wake(ctx.id()).await;
            return Ok(Routed::Handled);
        }
        let stored = self.load(ctx.id()).await?;
        let text = render::negotiator_bound(&view, self.today());
        self.to_negotiator(&stored, role, &ctx.room, &text).await
    }

    /// The roster name matching the sender's display name; else the one desk agent the bind
    /// wake addressed. Names are matched whole, ignoring case.
    async fn negotiator_name(
        &self,
        msg: &Inbound,
        desk: &RoomId,
    ) -> Result<Option<String>, AppError> {
        let names = self.desk_agents(desk).await?;
        let named = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case(msg.sender_name.trim()))
            .cloned();
        Ok(named.or(match names.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        }))
    }

    pub(super) async fn sign(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        hash8: String,
    ) -> Result<Routed, AppError> {
        if ctx.place != Place::DealRoom {
            return self.refuse(ctx, DomainError::WrongRoom.into()).await;
        }
        let cmd = RecordSignature::Typed {
            deal: ctx.id().clone(),
            signer: msg.sender.clone(),
            hash: hash8,
            message_id: MessageId::new(msg.message_id.as_str())?,
        };
        match record_signature(self.p.repo(), self.p.clock(), self.p.signer(), cmd).await {
            Ok(outcome) => self.after_sign(ctx.id(), outcome).await,
            Err(e) => self.refuse(ctx, e).await,
        }
    }

    pub(super) async fn withdraw(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        role: Role,
    ) -> Result<Routed, AppError> {
        if !matches!(ctx.place, Place::DealRoom) && ctx.place != Place::Desk(role) {
            return self.refuse(ctx, DomainError::WrongRoom.into()).await;
        }
        let cmd = Withdraw {
            deal: ctx.id().clone(),
            sender: msg.sender.clone(),
        };
        match withdraw(self.p.repo(), self.p.clock(), cmd).await {
            Ok(by) => {
                let deal = &ctx.stored.deal;
                self.post(deal.room(), &render::withdrawn(deal.id(), deal.kind(), by))
                    .await
            }
            Err(e) => self.refuse(ctx, e).await,
        }
    }

    pub(super) async fn desk_status(&self, ctx: &Ctx, role: Role) -> Result<Routed, AppError> {
        let status = get_status(self.p.repo(), ctx.id(), Some(role)).await?;
        let last = status.last_checks.and_then(|c| c.last().cloned());
        let verdict = last.map(|c| OwnVerdict {
            within: c.within,
            failing_fields: c.failing_fields,
        });
        let probes = u32::from(status.probes_left.unwrap_or(0));
        let text = render::desk_status(&ctx.stored.deal, probes, verdict.as_ref());
        self.post(&ctx.room, &text).await
    }

    pub(super) async fn state(&self, ctx: &Ctx, role: Role) -> Result<Routed, AppError> {
        let view = get_party_view(self.p.repo(), ctx.id(), role).await?;
        let text = format!("state\n{}", render::state_block(&view, self.today()));
        self.to_negotiator(&ctx.stored, role, &ctx.room, &text)
            .await
    }

    pub(super) async fn kinds(&self, ctx: &Ctx, role: Role) -> Result<Routed, AppError> {
        let text = format!("kinds\n{}", render::kinds());
        self.to_negotiator(&ctx.stored, role, &ctx.room, &text)
            .await
    }

    /// Today's UTC calendar day, from the notary clock.
    pub(super) fn today(&self) -> Date {
        Date::containing(self.p.clock().now())
    }

    /// `wake_turn` after a committed move: a failure is logged, never returned.
    pub(super) async fn wake(&self, id: &DealId) {
        if let Err(e) = self.wake_turn(id).await {
            eprintln!("turn wake for {id} failed: {e}");
        }
    }

    /// Wakes the negotiator whose turn it is, in its own desk room, with its `state` block,
    /// mentioning only the name recorded at its bind (or, for a side bound at open, the one
    /// agent in its desk). Does nothing unless the deal is open. Main calls this once both
    /// sides have sealed.
    pub async fn wake_turn(&self, id: &DealId) -> Result<(), AppError> {
        let stored = self.load(id).await?;
        let deal = &stored.deal;
        if deal.state() != DealState::Open {
            return Ok(());
        }
        let role = deal.turn();
        let Some(desk) = deal.desk(role) else {
            return Ok(());
        };
        let name = self
            .wake_name(&stored, role, desk)
            .await?
            .ok_or(AppError::Internal("no single negotiator name to wake"))?;
        let view = get_party_view(self.p.repo(), id, role).await?;
        self.post(desk, &render::wake_turn(&name, &view, self.today()))
            .await?;
        Ok(())
    }

    /// The name recorded at the side's bind, else the one agent in its desk.
    pub(super) async fn wake_name(
        &self,
        stored: &proxenos_app::stored::StoredDeal,
        role: Role,
        desk: &RoomId,
    ) -> Result<Option<String>, AppError> {
        if let Some(name) = stored.party(role).negotiator_name.clone() {
            return Ok(Some(name));
        }
        Ok(match self.desk_agents(desk).await?.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        })
    }

    /// Agent names in a desk room other than the notary: by template, just the negotiator.
    async fn desk_agents(&self, desk: &RoomId) -> Result<Vec<String>, AppError> {
        let names = self.p.roster().agent_names(desk).await?;
        Ok(names
            .into_iter()
            .filter(|n| !n.eq_ignore_ascii_case(&self.cfg.notary_name))
            .collect())
    }

    pub(super) async fn load(
        &self,
        id: &DealId,
    ) -> Result<proxenos_app::stored::StoredDeal, AppError> {
        self.p
            .repo()
            .load(id)
            .await?
            .ok_or_else(|| AppError::DealNotFound(id.clone()))
    }
}
