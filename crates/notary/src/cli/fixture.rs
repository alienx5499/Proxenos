//! `proxenos fixture D-0008 --round 3`: seals scripted limits, binds both parties and plays
//! scripted offers through the use cases, ending on the seller's $24.50/kg with the buyer to move.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proxenos_domain::date::Date;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::{DealKind, Field};
use proxenos_domain::limit::{Constraint, LimitSet};
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::party::{ParticipantId, Role};
use proxenos_domain::quantity::Quantity;
use proxenos_domain::terms::FieldValue;

use super::fixture_play::{Ports, play};
use super::{CliError, read_secret};
use crate::config::Secret;

/// Scripted numbers, US cents per kg.
pub const BUYER_MAX: i64 = 2_280;
pub const SELLER_MIN: i64 = 2_140;
pub const SELLER_LAST: i64 = 2_450;
const BUYER_OPEN: i64 = 2_050;
/// Delivery bounds and the offered day, in days after today.
const BUYER_DELIVER_BY: i32 = 40;
const SELLER_DELIVER_FROM: i32 = 29;
pub(super) const OFFER_DELIVER: i32 = 35;

/// Seal tokens are read from `--buyer-seal-file` / `--seller-seal-file` (`-` for stdin) or
/// `FIXTURE_BUYER_SEAL` / `FIXTURE_SELLER_SEAL`, never from argv. Re-running resumes: a side
/// already sealed must have no seal token given, its bindings must match, and offers already in
/// the log must match the script; anything else stops with the divergence named.
///
/// `--negotiator <id>` binds a spare buyer agent (e.g. `buyer-agent-2`) instead of
/// `--buyer-agent`. A negotiator cannot be rebound, so after a wedge run it on a fresh deal from
/// the group template; on a deal whose buyer negotiator is already bound to another id it stops.
#[derive(Debug, clap::Args)]
pub struct FixtureArgs {
    pub deal: String,
    #[arg(long)]
    pub round: u16,
    /// Participant id of a spare buyer negotiator, used instead of `--buyer-agent`.
    #[arg(long)]
    pub negotiator: Option<String>,
    #[arg(long)]
    pub buyer_seal_file: Option<PathBuf>,
    #[arg(long)]
    pub seller_seal_file: Option<PathBuf>,
    #[arg(long, env = "FIXTURE_BUYER_OWNER_ID")]
    pub buyer_owner: String,
    #[arg(long, env = "FIXTURE_SELLER_OWNER_ID")]
    pub seller_owner: String,
    #[arg(long, env = "FIXTURE_BUYER_AGENT_ID")]
    pub buyer_agent: String,
    #[arg(long, env = "FIXTURE_SELLER_AGENT_ID")]
    pub seller_agent: String,
}

impl FixtureArgs {
    pub fn buyer_negotiator(&self) -> &str {
        self.negotiator.as_deref().unwrap_or(&self.buyer_agent)
    }

    pub fn plan(&self) -> Result<Plan, CliError> {
        let id = |s: &str| ParticipantId::new(s).map_err(CliError::new);
        Ok(Plan {
            deal: self.deal.parse().map_err(CliError::new)?,
            round: self.round,
            seals: [
                read_secret("FIXTURE_BUYER_SEAL", self.buyer_seal_file.as_deref())?,
                read_secret("FIXTURE_SELLER_SEAL", self.seller_seal_file.as_deref())?,
            ],
            owners: [id(&self.buyer_owner)?, id(&self.seller_owner)?],
            agents: [id(self.buyer_negotiator())?, id(&self.seller_agent)?],
        })
    }
}

/// Index 0 is the buyer (role A), 1 the seller (role B). A seal is `None` for a side
/// already sealed on a resumed run.
pub struct Plan {
    pub deal: DealId,
    pub round: u16,
    pub seals: [Option<Secret>; 2],
    pub owners: [ParticipantId; 2],
    pub agents: [ParticipantId; 2],
}

fn usd(cents: i64) -> FieldValue {
    FieldValue::Money(Money::new(cents, Currency::Usd))
}

pub fn scripted_limits(role: Role, today: i32) -> LimitSet {
    use Constraint::*;
    let kg = |v| FieldValue::Quantity(Quantity::kg(v));
    let day = |d| Date::from_days(today + d);
    let map = match role {
        Role::A => [
            (Field::PricePerKg, AtMost(usd(BUYER_MAX))),
            (Field::QuantityKg, Between(kg(300), kg(500))),
            (Field::DeliverBy, NoLaterThan(day(BUYER_DELIVER_BY))),
            (Field::PaymentNetDays, AtLeast(FieldValue::Integer(30))),
        ],
        Role::B => [
            (Field::PricePerKg, AtLeast(usd(SELLER_MIN))),
            (Field::QuantityKg, Between(kg(200), kg(600))),
            (Field::DeliverBy, NoEarlierThan(day(SELLER_DELIVER_FROM))),
            (Field::PaymentNetDays, AtMost(FieldValue::Integer(45))),
        ],
    };
    LimitSet::new(DealKind::PurchaseOrder, role, BTreeMap::from(map))
        .expect("scripted limits are valid")
}

/// Offers so the deal sits at `round` with the seller's offer last: 2N-1 when the seller
/// opens (O-1..O-(2N-1)); with the buyer opening, 2N-2. Each is `(by, cents per kg)`.
pub fn scripted_prices(opener: Role, round: u16) -> Vec<(Role, i64)> {
    let n = usize::from(round) * 2;
    let count = if opener == Role::B {
        n - 1
    } else {
        n.saturating_sub(2)
    };
    let by = |i: usize| {
        if i.is_multiple_of(2) {
            opener
        } else {
            opener.other()
        }
    };
    let mut buyer_moves = 0;
    (0..count)
        .map(|i| {
            let role = by(i);
            let price = match role {
                Role::A => {
                    buyer_moves += 1;
                    (BUYER_OPEN + 72 * (buyer_moves - 1)).min(BUYER_MAX)
                }
                Role::B => {
                    let later = (i + 1..count).filter(|j| by(*j) == Role::B).count() as i64;
                    SELLER_LAST + 48 * later
                }
            };
            (role, price)
        })
        .collect()
}

/// Production run against the notary's database and Switch identity.
pub async fn run(args: &FixtureArgs, cfg: &crate::config::Config) -> Result<String, CliError> {
    use crate::adapters::SystemClock;
    use crate::adapters::switch::SwitchAdapter;
    use crate::crypto::secrets::OsSecrets;
    use crate::store::sqlite::Store;
    use crate::vault::aes::{AesVault, VaultKey};
    use proxenos_switch_client::{Credentials, SwitchClient};

    let plan = args.plan()?;
    let repo = Store::open(Path::new(&cfg.db))
        .await
        .map_err(CliError::new)?;
    let vault =
        AesVault::new(&VaultKey::parse(Some(cfg.vault_key.expose())).map_err(CliError::new)?);
    let secrets = OsSecrets::parse(Some(cfg.secret_key.expose())).map_err(CliError::new)?;
    let creds = Credentials {
        agent_id: cfg.agent_id.clone(),
        api_key: cfg.agent_key.expose().to_owned(),
    };
    let client = SwitchClient::new(&cfg.switch_url, creds).map_err(CliError::new)?;
    let switch = SwitchAdapter::new(client, None);
    let mut excluded = vec![ParticipantId::new(cfg.notary_sender.clone()).map_err(CliError::new)?];
    for r in &cfg.relay_ids {
        excluded.push(ParticipantId::new(r.clone()).map_err(CliError::new)?);
    }
    let ports = Ports {
        repo: &repo,
        vault: &vault,
        clock: &SystemClock,
        secrets: &secrets,
        switch: &switch,
        excluded,
    };
    play(ports, plan).await
}
