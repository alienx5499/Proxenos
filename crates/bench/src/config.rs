//! Live-stack settings from flags or environment.

use std::time::Duration;

use proxenos_switch_client::{Credentials, SwitchClient};

use crate::BenchError;
use crate::cli::{CliKind, CommandCli};
use crate::prereg::Side;

/// Live-stack settings; each is checked only when a run needs it.
#[derive(clap::Args, Clone, Default)]
pub struct Live {
    #[arg(long, env = "SWITCH_URL")]
    pub switch_url: Option<String>,
    /// Store ids (`/agents/{id}`): credentials and addressing policies only.
    #[arg(long, env = "BENCH_BUYER_AGENT_ID")]
    pub buyer_agent_id: Option<String>,
    #[arg(long, env = "BENCH_BUYER_AGENT_KEY", hide_env_values = true)]
    pub buyer_agent_key: Option<String>,
    #[arg(long, env = "BENCH_SELLER_AGENT_ID")]
    pub seller_agent_id: Option<String>,
    #[arg(long, env = "BENCH_SELLER_AGENT_KEY", hide_env_values = true)]
    pub seller_agent_key: Option<String>,
    /// The `sender` id on each negotiator's messages; the notary binds and matches these.
    #[arg(long, env = "BENCH_BUYER_SENDER_ID")]
    pub buyer_sender_id: Option<String>,
    #[arg(long, env = "BENCH_SELLER_SENDER_ID")]
    pub seller_sender_id: Option<String>,
    /// Owners' Gateway usernames, for admitting them to the notary's party rule.
    #[arg(long, env = "BENCH_BUYER_OWNER_ID")]
    pub buyer_owner_id: Option<String>,
    #[arg(long, env = "BENCH_SELLER_OWNER_ID")]
    pub seller_owner_id: Option<String>,
    /// The `sender` id on each owner's messages, bound as the side's owner.
    #[arg(long, env = "BENCH_BUYER_OWNER_SENDER_ID")]
    pub buyer_owner_sender_id: Option<String>,
    #[arg(long, env = "BENCH_SELLER_OWNER_SENDER_ID")]
    pub seller_owner_sender_id: Option<String>,
    #[arg(long, env = "BENCH_INJECTOR_AGENT_ID")]
    pub injector_agent_id: Option<String>,
    #[arg(long, env = "BENCH_INJECTOR_AGENT_KEY", hide_env_values = true)]
    pub injector_agent_key: Option<String>,
    /// Rooms for `--play`; bench runs ignore these and make fresh rooms per run.
    #[arg(long, env = "BENCH_DEAL_ROOM")]
    pub deal_room: Option<String>,
    #[arg(long, env = "BENCH_BUYER_DESK")]
    pub buyer_desk: Option<String>,
    #[arg(long, env = "BENCH_SELLER_DESK")]
    pub seller_desk: Option<String>,
    #[arg(long, env = "PROXENOS_URL")]
    pub notary_url: Option<String>,
    #[arg(long, env = "PROXENOS_OPERATOR_TOKEN", hide_env_values = true)]
    pub operator_token: Option<String>,
    /// The participant id the notary's messages carry (`sender`); state blocks come only from it.
    #[arg(long, env = "PROXENOS_NOTARY_SENDER_ID")]
    pub notary_sender: Option<String>,
    #[arg(long, env = "NOTARY_AGENT_ID")]
    pub notary_agent_id: Option<String>,
    #[arg(long, env = "SWITCH_GATEWAY_URL")]
    pub gateway_url: Option<String>,
    #[arg(long, env = "SWITCH_GATEWAY_KEY", hide_env_values = true)]
    pub gateway_key: Option<String>,
    #[arg(long, default_value = "buyer-agent")]
    pub buyer_name: String,
    #[arg(long, default_value = "seller-agent")]
    pub seller_name: String,
    #[arg(long, value_enum, default_value = "claude")]
    pub buyer_cli: CliKind,
    /// Seller CLI; both sides run Claude unless this is `codex`.
    #[arg(long, value_enum, default_value = "claude")]
    pub seller_cli: CliKind,
    #[arg(long, default_value_t = 300)]
    pub cli_timeout_secs: u64,
    /// Deal ids are D-(base + arm*1000 + attempt*100 + scenario number in the file);
    /// raise it to rerun everything on fresh ids.
    #[arg(long, default_value_t = 0)]
    pub deal_base: u32,
}

/// Secrets print as `<redacted>`.
impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secret = |v: &Option<String>| v.as_ref().map(|_| "<redacted>");
        f.debug_struct("Live")
            .field("switch_url", &self.switch_url)
            .field("buyer_agent_id", &self.buyer_agent_id)
            .field("buyer_agent_key", &secret(&self.buyer_agent_key))
            .field("seller_agent_id", &self.seller_agent_id)
            .field("seller_agent_key", &secret(&self.seller_agent_key))
            .field("injector_agent_id", &self.injector_agent_id)
            .field("injector_agent_key", &secret(&self.injector_agent_key))
            .field("deal_room", &self.deal_room)
            .field("notary_url", &self.notary_url)
            .field("operator_token", &secret(&self.operator_token))
            .field("notary_agent_id", &self.notary_agent_id)
            .field("notary_sender", &self.notary_sender)
            .field("gateway_url", &self.gateway_url)
            .field("gateway_key", &secret(&self.gateway_key))
            .finish_non_exhaustive()
    }
}

pub fn need<'a>(v: &'a Option<String>, name: &str) -> Result<&'a str, BenchError> {
    v.as_deref()
        .ok_or_else(|| BenchError::Input(format!("{name} is not set")))
}

impl Live {
    pub fn by_side<'a>(
        &'a self,
        side: Side,
        buyer: &'a Option<String>,
        seller: &'a Option<String>,
    ) -> &'a Option<String> {
        match side {
            Side::Buyer => buyer,
            Side::Seller => seller,
        }
    }

    pub fn client(&self, side: Side) -> Result<SwitchClient, BenchError> {
        let id = need(
            self.by_side(side, &self.buyer_agent_id, &self.seller_agent_id),
            "agent id",
        )?;
        let key = need(
            self.by_side(side, &self.buyer_agent_key, &self.seller_agent_key),
            "agent key",
        )?;
        connect(need(&self.switch_url, "SWITCH_URL")?, id, key)
    }

    pub fn name(&self, side: Side) -> &str {
        match side {
            Side::Buyer => &self.buyer_name,
            Side::Seller => &self.seller_name,
        }
    }

    pub fn cli(&self, side: Side) -> CommandCli {
        let kind = match side {
            Side::Buyer => self.buyer_cli,
            Side::Seller => self.seller_cli,
        };
        CommandCli {
            kind,
            timeout: Duration::from_secs(self.cli_timeout_secs),
        }
    }
}

pub fn connect(base: &str, id: &str, key: &str) -> Result<SwitchClient, BenchError> {
    Ok(SwitchClient::new(
        base,
        Credentials {
            agent_id: id.to_owned(),
            api_key: key.to_owned(),
        },
    )?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn debug_redacts_secrets() {
        let live = super::Live {
            buyer_agent_key: Some("sk-secret".into()),
            gateway_key: Some("gw-secret".into()),
            ..Default::default()
        };
        let shown = format!("{live:?}");
        assert!(!shown.contains("secret") && shown.contains("<redacted>"));
    }
}
