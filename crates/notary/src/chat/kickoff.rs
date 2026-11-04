//! Operator kickoffs: `open <kind> deal <id>` in the deal room, `desk <side> of <id>` in a desk.

use proxenos_app::AppError;
use proxenos_app::commands::open_deal::{OpenDeal, RecordDesk, open_deal, record_desk};
use proxenos_app::ports::{Clock, DealReader};
use proxenos_domain::deal::DEFAULT_OPENER;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::RoomId;
use proxenos_domain::timestamp::Timestamp;

use super::grammar::SideLabel;
use super::inbound::Inbound;
use super::render;
use super::replies;
use super::router::{Ports, Routed, Router};

/// A desk kickoff held until its deal is opened (the template posts all three at once).
/// Held in memory only: run the template after the notary has started, and re-run it if the
/// notary restarts between a desk kickoff and its open kickoff.
pub(super) struct PendingDesk {
    deal: DealId,
    side: SideLabel,
    room: RoomId,
    at: Timestamp,
}

/// A held desk kickoff whose open never came is dropped after this long.
const PENDING_DESK_SECS: i64 = 10 * 60;

impl<P: Ports> Router<P> {
    pub(super) async fn open(
        &self,
        msg: &Inbound,
        kind: DealKind,
        id: DealId,
        opener: Option<SideLabel>,
    ) -> Result<Routed, AppError> {
        let opener = match opener.map(|side| side.role(kind)) {
            None => DEFAULT_OPENER,
            Some(Some(role)) => role,
            Some(None) => return self.post(&msg.room, &replies::side_mismatch(kind)).await,
        };
        let p = &self.p;
        let now = p.clock().now();
        let cmd = OpenDeal {
            id: id.clone(),
            kind,
            room: msg.room.clone(),
            deadline: Timestamp(now.0.saturating_add(self.cfg.deal_secs)),
            max_rounds: self.cfg.max_rounds,
            opener,
            parties: None,
        };
        let tokens = match open_deal(p.repo(), p.clock(), p.secrets(), cmd).await {
            Ok(t) => t,
            Err(e) => {
                let text = replies::refusal_line(&e).ok_or(e)?;
                return self.post(&msg.room, &text).await;
            }
        };
        self.write_sheet(&id).await;
        self.post(&msg.room, &render::deal_open(&id, kind)).await?;
        let pending: Vec<PendingDesk> = {
            let mut all = self.pending_desks.lock().expect("pending desks lock");
            let (mine, rest) = all.drain(..).partition(|d| d.deal == id);
            *all = rest;
            mine.into_iter()
                .filter(|d: &PendingDesk| now.0 - d.at.0 < PENDING_DESK_SECS)
                .collect()
        };
        for d in pending {
            // The deal is committed: a failed desk record must not lose the seal tokens.
            if let Err(e) = self.record(&d.deal, kind, d.side, &d.room).await {
                eprintln!("held desk for {} not recorded: {e}", d.deal);
            }
        }
        Ok(Routed::Opened {
            deal: id,
            seal_tokens: tokens,
        })
    }

    pub(super) async fn desk(
        &self,
        msg: &Inbound,
        side: SideLabel,
        id: DealId,
    ) -> Result<Routed, AppError> {
        match self.p.repo().load(&id).await? {
            Some(stored) => self.record(&id, stored.deal.kind(), side, &msg.room).await,
            None => {
                let at = self.p.clock().now();
                let mut pending = self.pending_desks.lock().expect("pending desks lock");
                pending.retain(|d| {
                    !(d.deal == id && d.side == side) && at.0 - d.at.0 < PENDING_DESK_SECS
                });
                pending.push(PendingDesk {
                    deal: id,
                    side,
                    room: msg.room.clone(),
                    at,
                });
                Ok(Routed::Handled)
            }
        }
    }

    /// Refuses a side label that does not fit the deal's kind; `a` and `b` always fit.
    async fn record(
        &self,
        id: &DealId,
        kind: DealKind,
        side: SideLabel,
        room: &RoomId,
    ) -> Result<Routed, AppError> {
        let Some(role) = side.role(kind) else {
            return self.post(room, &replies::side_mismatch(kind)).await;
        };
        let cmd = RecordDesk {
            deal: id.clone(),
            role,
            room: room.clone(),
        };
        let text = match record_desk(self.p.repo(), self.p.clock(), cmd).await {
            Ok(()) => render::desk_recorded(id, kind, role),
            Err(e) => replies::refusal_line(&e).ok_or(e)?,
        };
        self.post(room, &text).await
    }
}
