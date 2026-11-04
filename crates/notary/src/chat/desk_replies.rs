//! Replies to a desk's bound negotiator, in its desk: every one starts with `@<name>`, so the
//! agent (which acts only on addressed messages) sees it. Error wakes are capped per round.

use std::collections::HashMap;
use std::sync::Mutex;

use proxenos_app::AppError;
use proxenos_app::queries::get_party_view::get_party_view;
use proxenos_app::stored::StoredDeal;
use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::{Role, RoomId};

use super::render;
use super::router::{Ctx, Place, Ports, Routed, Router};

/// Error wakes one negotiator may get in one round. The next refusal gets one hold wake (repeat
/// the last accepted offer or accept), and later ones are plain replies, so agents cannot loop.
const MAX_ERROR_WAKES: u8 = 6;

/// What a refusal to a negotiator becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// A wake with the error and the state block.
    Error,
    /// The one wake after the cap: the agent is at its limit, so it should hold or accept.
    Hold,
    /// A plain reply that wakes no one.
    Quiet,
}

/// Error wakes sent per (deal, side) in the round they were counted in. In memory, per
/// router: a restart only grants a fresh allowance. A round is one offer from each side, so
/// a new round starts the count again.
#[derive(Default)]
pub struct ErrorWakes(Mutex<HashMap<(DealId, Role), (u16, u8)>>);

impl ErrorWakes {
    /// Counts one refusal for `role` in `round` and says how to deliver it.
    pub fn next(&self, deal: &DealId, role: Role, round: u16) -> Wake {
        let mut map = self.0.lock().expect("error wake lock");
        let (r, n) = map.entry((deal.clone(), role)).or_insert((round, 0));
        if *r != round {
            (*r, *n) = (round, 0);
        }
        *n = n.saturating_add(1);
        match *n {
            n if n <= MAX_ERROR_WAKES => Wake::Error,
            n if n == MAX_ERROR_WAKES + 1 => Wake::Hold,
            _ => Wake::Quiet,
        }
    }
}

impl<P: Ports> Router<P> {
    /// Posts a desk error block in the desk: an error wake when the sender is that desk's
    /// bound negotiator, the plain block for anyone else.
    pub(super) async fn desk_refusal(&self, ctx: &Ctx, block: String) -> Result<Routed, AppError> {
        match ctx.place {
            Place::Desk(r) if ctx.stored.deal.negotiator_role(&ctx.sender) == Some(r) => {
                self.error_wake(&ctx.stored, r, &ctx.room, block).await
            }
            _ => self.post(&ctx.room, &block).await,
        }
    }

    /// An error for `role`'s negotiator in its `desk`: a wake with the `state` block while the
    /// deal is open, so it can retry in one turn; the plain block once over the cap.
    pub(super) async fn error_wake(
        &self,
        stored: &StoredDeal,
        role: Role,
        desk: &RoomId,
        block: String,
    ) -> Result<Routed, AppError> {
        let deal = &stored.deal;
        let Some(name) = self.name_or_log(stored, role, desk).await else {
            return self.post(desk, &block).await;
        };
        let wake = self.error_wakes.next(deal.id(), role, deal.round());
        if wake == Wake::Quiet {
            return self.post(desk, &block).await;
        }
        let state = if deal.state() == DealState::Open {
            let view = get_party_view(self.p.repo(), deal.id(), role).await?;
            Some(render::state_block(&view, self.today()))
        } else {
            None
        };
        let text = match wake {
            Wake::Hold => render::wake_hold(&name, &block, state),
            _ => render::wake_refused(&name, &block, state),
        };
        self.post(desk, &text).await
    }

    /// A direct answer to `role`'s negotiator in its `desk`, as `@<name> <text>`. Not capped:
    /// one per command the agent sent, so answers cannot loop.
    pub(super) async fn to_negotiator(
        &self,
        stored: &StoredDeal,
        role: Role,
        desk: &RoomId,
        text: &str,
    ) -> Result<Routed, AppError> {
        match self.name_or_log(stored, role, desk).await {
            Some(name) => self.post(desk, &render::wake(&name, text)).await,
            None => self.post(desk, text).await,
        }
    }

    /// `wake_name`, with a roster failure logged as no name (the reply then goes out plain).
    async fn name_or_log(&self, stored: &StoredDeal, role: Role, desk: &RoomId) -> Option<String> {
        self.wake_name(stored, role, desk)
            .await
            .unwrap_or_else(|e| {
                eprintln!("no negotiator name in {desk}: {e}");
                None
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_error_wakes_per_side_and_round_with_one_hold_wake() {
        let w = ErrorWakes::default();
        let id: DealId = "D-0007".parse().unwrap();
        assert!((0..MAX_ERROR_WAKES).all(|_| w.next(&id, Role::A, 1) == Wake::Error));
        assert_eq!(
            w.next(&id, Role::A, 1),
            Wake::Hold,
            "one hold wake after the cap"
        );
        assert_eq!(
            w.next(&id, Role::A, 1),
            Wake::Quiet,
            "then quiet, so it cannot loop"
        );
        assert_eq!(
            w.next(&id, Role::B, 1),
            Wake::Error,
            "the other side has its own count"
        );
        assert_eq!(w.next(&id, Role::A, 2), Wake::Error, "a new round resets");
    }

    #[test]
    fn hold_wake_tells_the_agent_to_repeat_or_accept() {
        let t = render::wake_hold("buyer-agent", "BLOCK", None);
        assert!(t.starts_with("@buyer-agent"));
        assert!(t.contains("repeat your last offer that went through") && t.contains("BLOCK"));
    }
}
