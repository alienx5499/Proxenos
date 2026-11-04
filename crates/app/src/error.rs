//! AppError: domain refusals, port failures and use-case refusals.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use thiserror::Error;

use crate::ports::{RepoError, SwitchError, VaultError};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error(transparent)]
    Switch(#[from] SwitchError),
    #[error("no deal {0}")]
    DealNotFound(DealId),
    #[error("{0} is already open")]
    DealExists(DealId),
    #[error("this room is already used by live deal {0}")]
    RoomInUse(DealId),
    #[error("unknown or used seal link")]
    UnknownSealToken,
    #[error("no join code is waiting in this room")]
    UnknownJoinCode,
    #[error("wrong join code, {attempts_left} attempts left")]
    WrongJoinCode { attempts_left: u8 },
    #[error("join is disabled for this side after too many wrong codes; ask the operator")]
    JoinLocked,
    #[error("that participant cannot be a negotiator")]
    ExcludedNegotiator,
    #[error("admitting needs exactly two owners and two agents")]
    AdmitNeedsTwoEach,
    #[error("unknown or closed sign card")]
    UnknownSignRequest,
    #[error("internal: {0}")]
    Internal(&'static str),
    #[error("these limits are for the other side")]
    WrongSide,
    #[error("probes are answered only in your own desk room")]
    ProbeOutsideDesk,
    #[error("no probes left this round")]
    ProbeBudgetSpent,
}
