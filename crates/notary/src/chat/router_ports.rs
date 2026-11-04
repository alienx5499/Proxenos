//! The router's ports bundle, idempotency port, configuration and outcome types.

use std::future::Future;

use proxenos_app::commands::open_deal::SealTokens;
use proxenos_app::ports::{
    Clock, DealRepository, LimitVaultPort, ReceiptSigner, RepoError, RosterPort, SecretSource,
    SwitchPort,
};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::ParticipantId;
use proxenos_domain::timestamp::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Duplicate,
    Unaddressed,
    OwnMessage,
    Refusal,
    /// A Switch availability auto-reply or join greeting posted as an agent.
    AutoReply,
    /// Text from an agent bound to no side of the deal that is not a command.
    UnboundAgentText,
    /// A platform message that is not a kickoff made on the operator's behalf.
    Platform,
    /// A kickoff verb from someone other than the operator's template.
    KickoffNotFromOperator,
    /// The room belongs to no deal.
    NoDeal,
    /// The sender is over the per-party rate limit.
    RateLimited,
}

pub enum Routed {
    Dropped(Dropped),
    Handled,
    /// The seal links are the caller's to deliver: they are never posted in a room
    /// the parties' agents can read.
    Opened {
        deal: DealId,
        seal_tokens: SealTokens,
    },
}

/// Idempotency on message ids; the SQLite store's `mark_seen` backs it in production.
pub trait SeenMessages: Send + Sync {
    /// True the first time `message_id` is seen, false on every redelivery.
    fn mark_seen(
        &self,
        message_id: &str,
        at: Timestamp,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;
}

/// Every port the router drives.
pub trait Ports: Send + Sync {
    type Repo: DealRepository;
    type Vault: LimitVaultPort;
    type Clock: Clock;
    type Secrets: SecretSource;
    type Signer: ReceiptSigner;
    type Switch: SwitchPort;
    type Roster: RosterPort;
    type Seen: SeenMessages;
    fn repo(&self) -> &Self::Repo;
    fn vault(&self) -> &Self::Vault;
    fn clock(&self) -> &Self::Clock;
    fn secrets(&self) -> &Self::Secrets;
    fn signer(&self) -> &Self::Signer;
    fn switch(&self) -> &Self::Switch;
    fn roster(&self) -> &Self::Roster;
    fn seen(&self) -> &Self::Seen;
}

pub struct RouterConfig {
    /// The notary's own participant id; never a negotiator, its own posts are dropped.
    pub notary: ParticipantId,
    /// The notary's agent name, left out when picking whom to wake from a desk roster.
    pub notary_name: String,
    /// The operator's display name, as Switch puts it in a kickoff's `on_behalf_of`;
    /// compared exactly. Switch has already admitted the kickoff under the notary's
    /// owner-only rule, so this is a second check.
    pub operator: String,
    /// Registered relay ids; never negotiators.
    pub relays: Vec<ParticipantId>,
    /// Open one Switch sign card per side at agreement (only if live check L16 passed).
    pub sign_cards: bool,
    /// Deal lifetime from the open kickoff, in seconds.
    pub deal_secs: i64,
    pub max_rounds: u16,
    /// Base of the notary's HTTP API, for the receipt link posted at Bound.
    pub public_url: String,
}

#[cfg(test)]
impl RouterConfig {
    pub fn new(notary: ParticipantId, operator: impl Into<String>) -> Self {
        Self {
            notary,
            notary_name: "notary".into(),
            operator: operator.into(),
            relays: Vec::new(),
            sign_cards: false,
            deal_secs: 86_400,
            max_rounds: 8,
            public_url: "http://notary.test".into(),
        }
    }
}
