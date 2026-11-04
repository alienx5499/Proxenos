//! Negotiator moves: probe, offer, accept. Failing fields go only to the mover's own desk;
//! the deal room sees outcome-only lines. Every desk reply is addressed to the mover.

use proxenos_app::AppError;
use proxenos_app::commands::accept_offer::{AcceptOffer, AcceptOutcome, accept_offer};
use proxenos_app::commands::probe_own_limits::{ProbeOwnLimits, probe_own_limits};
use proxenos_app::commands::submit_offer::{OfferOutcome, SubmitOffer, submit_offer};
use proxenos_app::queries::get_party_view::get_party_view;
use proxenos_domain::error::DomainError;
use proxenos_domain::kind::Field;
use proxenos_domain::limit::OwnVerdict;
use proxenos_domain::offer::{Note, OfferId};
use proxenos_domain::party::Role;
use proxenos_domain::signature::MessageId;
use proxenos_domain::terms::Terms;

use super::grammar::ParseError;
use super::inbound::Inbound;
use super::render;
use super::replies;
use super::router::{Ctx, Place, Ports, Routed, Router};

impl<P: Ports> Router<P> {
    pub(super) async fn probe(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        role: Role,
        terms: Terms,
    ) -> Result<Routed, AppError> {
        if ctx.place != Place::Desk(role) {
            return self.post(&ctx.room, &replies::probe_refused()).await;
        }
        let cmd = ProbeOwnLimits {
            deal: ctx.id().clone(),
            sender: msg.sender.clone(),
            room: ctx.room.clone(),
            terms,
        };
        match probe_own_limits(self.p.repo(), self.p.vault(), self.p.clock(), cmd).await {
            Ok(o) => {
                let verdict = OwnVerdict {
                    within: o.within,
                    failing_fields: o.failing_fields,
                };
                let result = render::probe_result(&verdict, u32::from(o.probes_left));
                let view = get_party_view(self.p.repo(), ctx.id(), role).await?;
                let state = match view.my_turn {
                    true => format!("\n{}", render::state_block(&view, self.today())),
                    false => String::new(),
                };
                let text = format!("probe result\n{result}{state}");
                self.to_negotiator(&ctx.stored, role, &ctx.room, &text)
                    .await
            }
            Err(e) => self.refuse(ctx, e).await,
        }
    }

    pub(super) async fn offer(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        role: Role,
        terms: Terms,
        note: Note,
    ) -> Result<Routed, AppError> {
        let deal = &ctx.stored.deal;
        let own_last = deal.offers().iter().rev().find(|o| o.by == role);
        if own_last.is_some_and(|o| o.terms == terms) {
            let block = render::parse_error(&ParseError::RepeatedTerms);
            if ctx.place == Place::DealRoom {
                self.post(&ctx.room, &render::offer_refused()).await?;
                return self.error_to_desk(ctx, role, block).await;
            }
            return self.desk_refusal(ctx, block).await;
        }
        let cmd = SubmitOffer {
            deal: ctx.id().clone(),
            sender: msg.sender.clone(),
            room: ctx.room.clone(),
            terms,
            note,
            message_id: MessageId::new(msg.message_id.as_str())?,
        };
        let outcome = submit_offer(self.p.repo(), self.p.vault(), self.p.clock(), cmd).await;
        match outcome {
            Ok(OfferOutcome::Made { offer }) => {
                let card = render::offer_card(deal.kind(), &offer);
                self.post(deal.room(), &card).await?;
                let ack = format!("offer recorded\n{}", render::offer_recorded(offer.id));
                self.to_desk(ctx, role, &ack).await?;
                self.wake(ctx.id()).await;
                Ok(Routed::Handled)
            }
            Ok(OfferOutcome::RefusedInDesk { failing_fields }) => {
                self.refused_with_fields(ctx, role, failing_fields, render::offer_refused())
                    .await
            }
            Ok(OfferOutcome::RefusedInDealRoom { desk_detail }) => {
                let fields = desk_detail.for_desk();
                self.refused_with_fields(ctx, role, fields, render::offer_refused())
                    .await
            }
            Ok(OfferOutcome::Expired) => self.post(deal.room(), &replies::expired(ctx.id())).await,
            Err(e) => {
                self.move_refused(ctx, role, e, render::offer_refused())
                    .await
            }
        }
    }

    pub(super) async fn accept(
        &self,
        msg: &Inbound,
        ctx: &Ctx,
        role: Role,
        offer_id: OfferId,
    ) -> Result<Routed, AppError> {
        let deal = &ctx.stored.deal;
        let cmd = AcceptOffer {
            deal: ctx.id().clone(),
            sender: msg.sender.clone(),
            room: ctx.room.clone(),
            offer_id,
        };
        match accept_offer(self.p.repo(), self.p.vault(), self.p.clock(), cmd).await {
            Ok(AcceptOutcome::Agreed { hash }) => {
                let hash8 = hash.short();
                let agreed = self.load(ctx.id()).await?;
                let terms = &agreed
                    .deal
                    .agreed_offer()
                    .ok_or(AppError::Internal("agreed without an offer"))?
                    .terms;
                let card = render::terms_card(ctx.id(), terms, &hash8);
                self.post(deal.room(), &card).await?;
                let ack = format!("agreed\n{}", render::agreed(&hash8));
                self.to_desk(ctx, role, &ack).await?;
                if self.cfg.sign_cards {
                    self.open_sign_cards(ctx.id()).await;
                }
                Ok(Routed::Handled)
            }
            Ok(AcceptOutcome::RefusedInDesk { failing_fields }) => {
                self.refused_with_fields(ctx, role, failing_fields, render::accept_refused())
                    .await
            }
            Ok(AcceptOutcome::RefusedInDealRoom { desk_detail }) => {
                let fields = desk_detail.for_desk();
                self.refused_with_fields(ctx, role, fields, render::accept_refused())
                    .await
            }
            Err(e) => {
                self.move_refused(ctx, role, e, render::accept_refused())
                    .await
            }
        }
    }

    /// Outcome-only line in the deal room; own failing fields only in the mover's desk.
    async fn refused_with_fields(
        &self,
        ctx: &Ctx,
        role: Role,
        fields: Vec<Field>,
        line: String,
    ) -> Result<Routed, AppError> {
        self.post(ctx.stored.deal.room(), &line).await?;
        let block = render::desk_error(&DomainError::OutsideOwnLimits(fields));
        if ctx.place == Place::Desk(role) {
            return self.desk_refusal(ctx, block).await;
        }
        self.error_to_desk(ctx, role, block).await
    }

    /// From the deal room: the outcome line there, the error block in the mover's desk.
    async fn move_refused(
        &self,
        ctx: &Ctx,
        role: Role,
        err: AppError,
        line: String,
    ) -> Result<Routed, AppError> {
        if ctx.place == Place::DealRoom {
            let block = replies::desk_app_error(&err).ok_or(err)?;
            self.post(&ctx.room, &line).await?;
            self.error_to_desk(ctx, role, block).await
        } else {
            self.refuse(ctx, err).await
        }
    }

    /// An answer in the mover's desk, addressed to it.
    async fn to_desk(&self, ctx: &Ctx, role: Role, text: &str) -> Result<Routed, AppError> {
        match ctx.stored.deal.desk(role) {
            Some(desk) => self.to_negotiator(&ctx.stored, role, desk, text).await,
            None => Ok(Routed::Handled),
        }
    }

    /// An error block in the mover's desk, as a capped error wake.
    async fn error_to_desk(
        &self,
        ctx: &Ctx,
        role: Role,
        block: String,
    ) -> Result<Routed, AppError> {
        match ctx.stored.deal.desk(role) {
            Some(desk) => self.error_wake(&ctx.stored, role, desk, block).await,
            None => Ok(Routed::Handled),
        }
    }
}
