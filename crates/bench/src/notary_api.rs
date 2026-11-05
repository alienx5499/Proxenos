//! Calls the bench makes outside the chat: notary HTTP API and the Gateway addressing policy.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::BenchError;
use crate::config::{Live, need};
use crate::domain_json;
use crate::prereg::Side;
use crate::prereg::{Answer, Scenario};

/// `sender` ids, as the notary sees them on each party's messages.
#[derive(Clone, Debug, Serialize)]
pub struct PartyIds {
    pub owner_id: String,
    pub agent_id: String,
}

/// `POST /api/deals` body. Bench-only: parties are bound up front, so no `join` is typed.
#[derive(Clone, Debug, Serialize)]
pub struct OpenDeal {
    pub deal_id: String,
    pub kind: String,
    pub room_id: String,
    pub desks: BTreeMap<Side, String>,
    pub max_rounds: u32,
    pub parties: BTreeMap<Side, PartyIds>,
}

#[derive(Deserialize)]
struct Opened {
    seal_links: BTreeMap<Side, String>,
}

/// The fields of `GET /api/deals/{id}/timeline` the driver reads.
#[derive(Clone, Debug, Deserialize)]
pub struct DealStatus {
    pub state: String,
    pub round: u32,
    #[serde(default)]
    pub agreed_terms: Option<Value>,
}

#[derive(Clone)]
pub struct Http {
    pub(crate) http: reqwest::Client,
    base: String,
    token: String,
    /// The Gateway reads only its `switch_auth` login cookie, never a bearer token.
    cookie: bool,
}

impl Http {
    pub fn new(base: &str, token: &str) -> Result<Self, BenchError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(transport)?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            cookie: false,
        })
    }

    /// A client for the Switch Gateway: the key goes as the `switch_auth` cookie.
    pub fn gateway(base: &str, key: &str) -> Result<Self, BenchError> {
        Ok(Self {
            cookie: true,
            ..Self::new(base, key)?
        })
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.cookie {
            req.header(
                reqwest::header::COOKIE,
                format!("switch_auth={}", self.token),
            )
        } else {
            req.bearer_auth(&self.token)
        }
    }

    pub(crate) async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, BenchError> {
        let resp = self.authed(req).send().await.map_err(transport)?;
        let status = resp.status();
        let text = resp.text().await.map_err(transport)?;
        if !status.is_success() {
            return Err(BenchError::Http(format!("{status}: {text}")));
        }
        Ok(if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)?
        })
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Opens the deal and returns each side's seal token (the last segment of its link).
    pub async fn open_deal(&self, req: &OpenDeal) -> Result<BTreeMap<Side, String>, BenchError> {
        let v = self
            .send(self.http.post(self.url("/api/deals")).json(req))
            .await?;
        let opened: Opened = serde_json::from_value(v)?;
        Ok(opened
            .seal_links
            .into_iter()
            .map(|(side, link)| {
                (
                    side,
                    link.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect())
    }

    /// Seal tokens authenticate themselves; the operator token is not sent.
    pub async fn seal(&self, token: &str, limits: &Value) -> Result<(), BenchError> {
        let url = self.url(&format!("/api/seal/{token}"));
        let resp = self
            .http
            .post(url)
            .json(&json!({ "limits": limits }))
            .send()
            .await;
        let resp = resp.map_err(transport)?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(BenchError::Http(format!(
                "seal refused: {status}: {}",
                resp.text().await.unwrap_or_default()
            )));
        }
        Ok(())
    }

    /// `owners` are Gateway usernames, `agents` store ids: the notary's policy takes these,
    /// not `sender` ids.
    pub async fn admit(
        &self,
        deal_id: &str,
        owners: &[&str],
        agents: &[&str],
    ) -> Result<(), BenchError> {
        let url = self.url(&format!("/api/deals/{deal_id}/admit"));
        let body = json!({ "owners": owners, "agents": agents });
        self.send(self.http.post(url).json(&body)).await.map(drop)
    }

    pub async fn status(&self, deal_id: &str) -> Result<DealStatus, BenchError> {
        let v = self
            .send(
                self.http
                    .get(self.url(&format!("/api/deals/{deal_id}/timeline"))),
            )
            .await?;
        Ok(serde_json::from_value(v)?)
    }
}

/// Opens the deal through the operator API, seals both sides from the answer key, admits the parties.
pub async fn open_and_seal(
    live: &Live,
    deal_id: &str,
    deal_room: &str,
    sc: &Scenario,
    ans: &Answer,
    currency: &str,
) -> Result<Http, BenchError> {
    let api = Http::new(
        need(&live.notary_url, "PROXENOS_URL")?,
        need(&live.operator_token, "PROXENOS_OPERATOR_TOKEN")?,
    )?;
    let Roster {
        desks,
        parties,
        owners,
        agents,
    } = roster(live)?;
    let req = OpenDeal {
        deal_id: deal_id.to_owned(),
        kind: sc.kind.clone(),
        room_id: deal_room.to_owned(),
        desks,
        max_rounds: crate::driver::MAX_ROUNDS,
        parties,
    };
    let tokens = api.open_deal(&req).await?;
    seal_sides(&api, &tokens, sc, ans, currency).await?;
    api.admit(deal_id, &owners, &agents).await?;
    Ok(api)
}

/// Desk rooms and notary-visible sender ids per side, plus the Switch ids to admit.
struct Roster<'a> {
    desks: BTreeMap<Side, String>,
    parties: BTreeMap<Side, PartyIds>,
    owners: Vec<&'a str>,
    agents: Vec<&'a str>,
}

fn roster(live: &Live) -> Result<Roster<'_>, BenchError> {
    let mut desks = BTreeMap::new();
    let mut parties = BTreeMap::new();
    let (mut owners, mut agents) = (Vec::new(), Vec::new());
    for side in [Side::Buyer, Side::Seller] {
        let desk = live.by_side(side, &live.buyer_desk, &live.seller_desk);
        desks.insert(side, need(desk, "desk room")?.to_owned());
        let upper = side.to_string().to_ascii_uppercase();
        let pick = |buyer, seller, what: &str| {
            need(
                live.by_side(side, buyer, seller),
                &format!("BENCH_{upper}_{what}"),
            )
        };
        let (owner_sender, sender) = (
            pick(
                &live.buyer_owner_sender_id,
                &live.seller_owner_sender_id,
                "OWNER_SENDER_ID",
            )?,
            pick(&live.buyer_sender_id, &live.seller_sender_id, "SENDER_ID")?,
        );
        owners.push(pick(
            &live.buyer_owner_id,
            &live.seller_owner_id,
            "OWNER_ID",
        )?);
        agents.push(pick(
            &live.buyer_agent_id,
            &live.seller_agent_id,
            "AGENT_ID",
        )?);
        parties.insert(
            side,
            PartyIds {
                owner_id: owner_sender.to_owned(),
                agent_id: sender.to_owned(),
            },
        );
    }
    Ok(Roster {
        desks,
        parties,
        owners,
        agents,
    })
}

async fn seal_sides(
    api: &Http,
    tokens: &BTreeMap<Side, String>,
    sc: &Scenario,
    ans: &Answer,
    currency: &str,
) -> Result<(), BenchError> {
    for side in [Side::Buyer, Side::Seller] {
        let token = tokens
            .get(&side)
            .ok_or_else(|| BenchError::Http(format!("no seal link for {side}")))?;
        let limits = ans
            .limits
            .get(&side)
            .ok_or_else(|| BenchError::Input(format!("{} has no {side} limits", sc.id)))?;
        api.seal(token, &domain_json::limits(limits, currency)?)
            .await?;
    }
    Ok(())
}

/// Transport errors without the URL, which can carry a seal token.
fn transport(e: reqwest::Error) -> BenchError {
    BenchError::Http(e.without_url().to_string())
}

#[cfg(test)]
#[path = "notary_api_tests.rs"]
mod tests;
