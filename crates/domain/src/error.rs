//! DomainError with explicit variants.

use thiserror::Error;

use crate::kind::Field;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("amounts are in different currencies")]
    CurrencyMismatch,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("no limit for {0}")]
    MissingLimit(Field),
    #[error("{0} is not a field of this deal kind")]
    UnexpectedLimit(Field),
    #[error("wrong constraint direction for {0}")]
    WrongDirection(Field),
    #[error("wrong value type for {0}")]
    WrongValueType(Field),
    #[error("empty range for {0}")]
    EmptyRange(Field),

    #[error("{0} must not be empty")]
    EmptyId(&'static str),
    #[error("deal id must look like D-0007")]
    InvalidDealId,
    #[error("offer id must look like O-5")]
    InvalidOfferId,
    #[error("hash must be 64 lowercase hex characters")]
    InvalidHash,
    #[error("note is longer than 280 characters")]
    NoteTooLong,
    #[error("max rounds must be at least 1")]
    InvalidMaxRounds,

    #[error("waiting for both sides to seal limits")]
    AwaitingSeals,
    #[error("the deal is closed")]
    DealClosed,
    #[error("not your turn")]
    NotYourTurn,
    #[error("the deadline has passed")]
    DeadlinePassed,
    #[error("terms are for another deal kind")]
    KindMismatch,
    #[error("limits belong to another role or deal kind")]
    WrongLimitSet,
    #[error("outside own limits")]
    OutsideOwnLimits(Vec<Field>),
    #[error("only the counterparty's latest offer can be accepted")]
    NotLatestOffer,
    #[error("there are no agreed terms to sign")]
    NothingToSign,
    #[error("hash does not match the current terms")]
    HashMismatch,
    #[error("signer is not the bound owner")]
    NotBoundOwner,
    #[error("this side has already signed")]
    AlreadySigned,
    #[error("message id already used")]
    DuplicateMessage,
    #[error("limits already sealed")]
    AlreadySealed,
    #[error("desk room already recorded")]
    DeskAlreadyRecorded,
    #[error("no desk room recorded for this side")]
    DeskNotRecorded,
    #[error("wrong room")]
    WrongRoom,
    #[error("party already bound")]
    AlreadyBound,
    #[error("party not bound")]
    PartyNotBound,
    #[error("negotiator already bound")]
    NegotiatorAlreadyBound,
    #[error("an owner cannot also be a negotiator")]
    OwnerIsNegotiator,
    #[error("one participant cannot own both sides")]
    OwnerIsCounterparty,
    #[error("one participant cannot negotiate for both sides")]
    NegotiatorIsCounterparty,
    #[error("sender is not the bound negotiator")]
    NotBoundNegotiator,
    #[error("stored deal is inconsistent: {0}")]
    InvalidRecord(&'static str),
}
