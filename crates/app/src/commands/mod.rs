//! Commands: change state; may return their own outcome.

pub mod accept_offer;
pub mod admit_parties;
pub mod bind_negotiator;
pub mod bind_party;
pub mod expire_due;
pub mod open_deal;
pub mod open_sign_card;
pub mod probe_own_limits;
pub mod record_deal_sheet;
pub mod record_signature;
pub mod seal_limits;
pub mod submit_offer;
pub mod withdraw;

use proxenos_domain::deal::Deal;
use proxenos_domain::error::DomainError;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::party::{ParticipantId, Role, RoomId};

use crate::error::AppError;
use crate::ports::LimitVaultPort;
use crate::stored::StoredDeal;

/// A value only the mover's own desk may see. No `Serialize` and no `Display`, so it cannot
/// reach a deal-room reply or a log line by accident; the desk renderer calls `for_desk`.
#[derive(Clone, PartialEq, Eq)]
pub struct DeskOnly<T>(T);

impl<T> DeskOnly<T> {
    pub fn for_desk(self) -> T {
        self.0
    }
}

impl<T> std::fmt::Debug for DeskOnly<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeskOnly(..)")
    }
}

/// The side `sender` negotiates for.
fn negotiator_role(deal: &Deal, sender: &ParticipantId) -> Result<Role, AppError> {
    Ok(deal
        .negotiator_role(sender)
        .ok_or(DomainError::NotBoundNegotiator)?)
}

/// Offers and accepts come from the deal room or the mover's own desk room.
fn ensure_move_room(deal: &Deal, role: Role, room: &RoomId) -> Result<(), AppError> {
    if room == deal.room() || deal.desk(role) == Some(room) {
        Ok(())
    } else {
        Err(DomainError::WrongRoom.into())
    }
}

/// Decrypts `role`'s sealed limits from the loaded deal.
fn own_limits(
    vault: &impl LimitVaultPort,
    stored: &StoredDeal,
    role: Role,
) -> Result<LimitSet, AppError> {
    let sealed = stored
        .party(role)
        .sealed
        .as_ref()
        .ok_or(DomainError::AwaitingSeals)?;
    Ok(vault.decrypt(stored.deal.id(), role, sealed)?)
}

/// Whether failing fields may be returned: only to the mover's own desk room.
fn in_own_desk(deal: &Deal, role: Role, room: &RoomId) -> bool {
    deal.desk(role) == Some(room)
}
