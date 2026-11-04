//! Query: get_timeline, the public redacted log: no probes, no failing fields, no limits.

use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::{DealEvent, ExpiryReason};
use proxenos_domain::hash::TermsHash;
use proxenos_domain::kind::DealKind;
use proxenos_domain::offer::OfferId;
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;
use serde::Serialize;

use super::label;
use crate::error::AppError;
use crate::ports::DealReader;
use crate::stored::load_deal;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineEntry {
    pub seq: u64,
    pub at: Timestamp,
    #[serde(flatten)]
    pub item: TimelineItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItem {
    Opened {
        kind: DealKind,
        max_rounds: u16,
        deadline: Timestamp,
    },
    Sealed {
        by: String,
    },
    Joined {
        by: String,
    },
    NegotiatorBound {
        by: String,
    },
    OfferMade {
        id: OfferId,
        by: String,
        round: u16,
        terms: Terms,
    },
    OfferRefused {
        by: String,
    },
    AcceptRefused {
        by: String,
    },
    Agreed {
        by: String,
        offer_id: OfferId,
        terms_hash: TermsHash,
    },
    Signed {
        by: String,
    },
    Bound {
        terms_hash: TermsHash,
    },
    Withdrawn {
        by: String,
    },
    Expired {
        reason: ExpiryReason,
    },
}

pub async fn get_timeline(
    repo: &impl DealReader,
    deal: &DealId,
) -> Result<Vec<TimelineEntry>, AppError> {
    let kind = load_deal(repo, deal).await?.deal.kind();
    let events = repo.events(deal).await?;
    Ok(events
        .into_iter()
        .filter_map(|e| {
            let item = public(kind, e.event)?;
            Some(TimelineEntry {
                seq: e.seq,
                at: e.at,
                item,
            })
        })
        .collect())
}

fn public(kind: DealKind, event: DealEvent) -> Option<TimelineItem> {
    use TimelineItem as T;
    let by = |r| label(kind, r);
    Some(match event {
        DealEvent::Opened {
            kind,
            max_rounds,
            deadline,
            ..
        } => T::Opened {
            kind,
            max_rounds,
            deadline,
        },
        DealEvent::DeskRecorded { .. } | DealEvent::Probed { .. } => return None,
        DealEvent::LimitsSealed { role } => T::Sealed { by: by(role) },
        DealEvent::PartyBound { role, .. } => T::Joined { by: by(role) },
        DealEvent::NegotiatorBound { role, .. } => T::NegotiatorBound { by: by(role) },
        DealEvent::OfferMade { offer } => T::OfferMade {
            id: offer.id,
            by: by(offer.by),
            round: offer.round,
            terms: offer.terms,
        },
        DealEvent::OfferRefused { role } => T::OfferRefused { by: by(role) },
        DealEvent::AcceptRefused { role } => T::AcceptRefused { by: by(role) },
        DealEvent::Accepted {
            by: r,
            offer_id,
            hash,
        } => T::Agreed {
            by: by(r),
            offer_id,
            terms_hash: hash,
        },
        DealEvent::Signed { signature } => T::Signed {
            by: by(signature.role),
        },
        DealEvent::Bound { hash } => T::Bound { terms_hash: hash },
        DealEvent::Withdrawn { by: r } => T::Withdrawn { by: by(r) },
        DealEvent::Expired { reason } => T::Expired { reason },
    })
}
