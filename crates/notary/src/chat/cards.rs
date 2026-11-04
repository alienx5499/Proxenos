//! Signatures: posting sign outcomes, sign cards (conditional core), and the deal sheet.

use proxenos_app::AppError;
use proxenos_app::commands::open_sign_card::{OpenSignCard, open_sign_card};
use proxenos_app::commands::record_deal_sheet::{mark_sheet_dirty, record_deal_sheet};
use proxenos_app::commands::record_signature::{RecordSignature, SignOutcome, record_signature};
use proxenos_app::ports::{DealReader, DocId, SignatureOutcomes, SwitchPort};
use proxenos_app::queries::get_timeline::get_timeline;
use proxenos_app::stored::SignRequestState;
use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::Role;

use super::render;
use super::replies;
use super::router::{Ports, Routed, Router};
use super::sheet;

impl<P: Ports> Router<P> {
    /// Posts a recorded sign act in the deal room; on Bound also rewrites the deal sheet.
    pub(super) async fn after_sign(
        &self,
        id: &DealId,
        outcome: SignOutcome,
    ) -> Result<Routed, AppError> {
        let stored = self.load(id).await?;
        let deal = &stored.deal;
        let (kind, room) = (deal.kind(), deal.room());
        let hash8 = match deal.state() {
            DealState::Agreed(h) | DealState::PartlySigned(h) | DealState::Bound(h) => h.short(),
            _ => String::new(),
        };
        match outcome {
            SignOutcome::Signed { role } => {
                self.post(room, &render::signed(kind, role, &hash8)).await
            }
            SignOutcome::Bound { role, .. } => {
                self.post(room, &render::signed(kind, role, &hash8)).await?;
                self.post(room, &replies::deal_bound(id, &hash8, &self.cfg.public_url))
                    .await?;
                self.write_sheet(id).await;
                Ok(Routed::Handled)
            }
            SignOutcome::Declined { role } => self.post(room, &replies::declined(kind, role)).await,
            SignOutcome::CardExpired { role } => {
                self.post(room, &replies::card_expired(kind, role)).await
            }
            SignOutcome::WrongParty { role } => {
                self.post(room, &replies::wrong_party()).await?;
                let wrong = stored
                    .sign_requests
                    .iter()
                    .filter(|r| r.role == role && r.state == SignRequestState::WrongParty)
                    .count();
                if wrong == 1 {
                    self.open_sign_card(id, role).await;
                }
                Ok(Routed::Handled)
            }
        }
    }

    pub(super) async fn open_sign_cards(&self, id: &DealId) {
        for role in [Role::A, Role::B] {
            self.open_sign_card(id, role).await;
        }
    }

    /// A card that fails to open is logged; the typed `sign` stays available.
    async fn open_sign_card(&self, id: &DealId, role: Role) {
        let cmd = OpenSignCard {
            deal: id.clone(),
            role,
        };
        if let Err(e) = open_sign_card(self.p.repo(), self.p.switch(), self.p.clock(), cmd).await {
            eprintln!("sign card for {id} {role:?} not opened: {e}");
        }
    }

    /// Writes the deal sheet after a committed move, never blocking it: a failed write is
    /// logged and the deal flagged, so `retry_dirty_sheets` writes it later.
    pub(super) async fn write_sheet(&self, id: &DealId) {
        if let Err(e) = self.try_write_sheet(id).await {
            eprintln!("deal sheet for {id} not written, will retry: {e}");
            if let Err(e) = mark_sheet_dirty(self.p.repo(), self.p.clock(), id).await {
                eprintln!("deal sheet for {id} not flagged for retry: {e}");
            }
        }
    }

    /// Rewrites every flagged deal sheet; tries all, returns the first failure.
    pub async fn retry_dirty_sheets(&self) -> Result<usize, AppError> {
        let mut first_err = None;
        let mut written = 0;
        for id in self.p.repo().dirty_sheets().await? {
            match self.try_write_sheet(&id).await {
                Ok(()) => written += 1,
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        first_err.map_or(Ok(written), Err)
    }

    async fn try_write_sheet(&self, id: &DealId) -> Result<(), AppError> {
        let stored = self.load(id).await?;
        let timeline = get_timeline(self.p.repo(), id).await?;
        let content = sheet::deal_sheet(&stored, &timeline);
        let room = stored.deal.room();
        let switch = self.p.switch();
        let doc = match stored.deal_sheet_doc.clone() {
            Some(doc) => {
                let doc = DocId(doc);
                switch.update_document(room, &doc, &content).await?;
                doc
            }
            None => {
                let name = format!("{id} deal sheet");
                switch.write_document(room, &name, &content).await?
            }
        };
        // Records a new document and clears the dirty flag; a no-op when neither changed.
        record_deal_sheet(self.p.repo(), self.p.clock(), id, &doc).await
    }
}

impl<P: Ports> Router<P>
where
    P::Switch: SignatureOutcomes,
{
    /// One poll: records each undelivered sign-card answer, posts the result, acks it, then
    /// retries flagged deal sheets. A store or Switch failure stops the poll without acking,
    /// so the next poll retries.
    pub async fn process_card_outcomes(&self) -> Result<usize, AppError> {
        let outcomes = self.p.switch().undelivered().await?;
        let count = outcomes.len();
        for outcome in outcomes {
            let request = outcome.request.clone();
            if let Some(id) = self.p.repo().by_sign_request(&request).await? {
                let cmd = RecordSignature::Card {
                    deal: id.clone(),
                    outcome,
                };
                let p = &self.p;
                match record_signature(p.repo(), p.clock(), p.signer(), cmd).await {
                    Ok(signed) => {
                        self.after_sign(&id, signed).await?;
                    }
                    Err(e) if replies::refusal_line(&e).is_some() => {
                        eprintln!("sign card {} for {id} refused: {e}", request.0);
                    }
                    Err(e) => return Err(e),
                }
            } else {
                eprintln!("sign card {} matches no open deal", request.0);
            }
            self.p.switch().ack_outcome(&request).await?;
        }
        self.retry_dirty_sheets().await?;
        Ok(count)
    }
}
