//! Role of a party, participant and room ids, and a party's binding.

use serde::{Deserialize, Serialize};

use crate::id::non_empty_id;

/// A is Buyer / Tenant, B is Seller / Landlord; labels come from `DealKind::role_label`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    A,
    B,
}

impl Role {
    pub fn other(self) -> Role {
        match self {
            Role::A => Role::B,
            Role::B => Role::A,
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Role::A => 0,
            Role::B => 1,
        }
    }
}

non_empty_id!(
    /// A chat sender id as Switch reports it (person or agent).
    ParticipantId
);

non_empty_id!(RoomId);

/// Owner is bound by the join code; the negotiator is bound later by its first command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyBinding {
    pub owner: ParticipantId,
    pub agent: Option<ParticipantId>,
    pub desk: RoomId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_blank() {
        assert!(ParticipantId::new("").is_err());
        assert!(RoomId::new("  ").is_err());
        assert_eq!(ParticipantId::new("u1").unwrap().as_str(), "u1");
        assert!(serde_json::from_str::<RoomId>("\"\"").is_err());
        let r: RoomId = serde_json::from_str("\"r1\"").unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"r1\"");
    }

    #[test]
    fn other_role_flips() {
        assert_eq!(Role::A.other(), Role::B);
        assert_eq!(Role::B.other(), Role::A);
    }
}
