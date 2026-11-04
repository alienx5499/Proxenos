//! The `D-0007 deal sheet` room document: status, public timeline, and the PO once bound.
//! Built from the redacted timeline only, so it carries no probe, failing field or limit.

use proxenos_app::queries::get_timeline::{TimelineEntry, TimelineItem};
use proxenos_app::stored::StoredDeal;

use super::format;
use super::render;

pub fn deal_sheet(stored: &StoredDeal, timeline: &[TimelineEntry]) -> String {
    let deal = &stored.deal;
    let mut out = format!(
        "# {} deal sheet\n{}\n\n## Timeline\n",
        deal.id(),
        render::status_line(deal)
    );
    for e in timeline {
        out.push_str(&format!(
            "- {} {}\n",
            format::time_of_day(e.at),
            item(&e.item)
        ));
    }
    if let Some(r) = &stored.receipt {
        let b = &r.body;
        out.push_str(&format!(
            "\n## {} {}\n{}\nterms hash `{}`\n",
            b.kind.name(),
            b.deal_id,
            format::terms(&b.terms),
            b.terms_hash
        ));
        for s in &b.signatures {
            let who = render::label(b.kind, s.role);
            out.push_str(&format!(
                "- signed by {who} at {}\n",
                format::time_of_day(s.at)
            ));
        }
        out.push_str(&format!("receipt signature `{}`\n", r.signature));
    }
    out.replace('@', "")
}

fn item(i: &TimelineItem) -> String {
    use TimelineItem as T;
    match i {
        T::Opened {
            kind, max_rounds, ..
        } => format!("opened · {} · {max_rounds} rounds", kind.name()),
        T::Sealed { by } => format!("{by} sealed limits"),
        T::Joined { by } => format!("{by} joined"),
        T::NegotiatorBound { by } => format!("{by} negotiator bound"),
        T::OfferMade {
            id,
            by,
            round,
            terms,
        } => {
            format!(
                "offer {id} by {by}, round {round}: {}",
                format::terms(terms)
            )
        }
        T::OfferRefused { by } => format!("offer by {by} refused by notary"),
        T::AcceptRefused { by } => format!("accept by {by} refused by notary"),
        T::Agreed {
            by,
            offer_id,
            terms_hash,
        } => {
            format!("{by} accepted {offer_id} · `{}`", terms_hash.short())
        }
        T::Signed { by } => format!("{by} signed"),
        T::Bound { terms_hash } => format!("bound · `{}`", terms_hash.short()),
        T::Withdrawn { by } => format!("withdrawn by {by}"),
        T::Expired { reason } => format!("expired ({reason:?})"),
    }
}
