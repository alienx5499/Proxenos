//! Plays a fixture plan through the use cases, resuming from whatever the deal already holds.

use proxenos_app::AppError;
use proxenos_app::commands::submit_offer::{OfferOutcome, SubmitOffer, submit_offer};
use proxenos_app::ports::{
    Clock, DealRepository, LimitVaultPort, RepoError, SecretSource, SwitchPort,
};
use proxenos_app::stored::StoredDeal;
use proxenos_domain::date::Date;
use proxenos_domain::kind::DealKind;
use proxenos_domain::money::{Currency, Money};
use proxenos_domain::offer::{Note, Offer};
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::quantity::Quantity;
use proxenos_domain::signature::MessageId;
use proxenos_domain::terms::{PurchaseOrderTerms, Terms};

use super::CliError;
use super::fixture::{OFFER_DELIVER, Plan, SELLER_LAST, scripted_prices};
use super::fixture_bind::seal_and_bind;
use crate::chat::{format, render};

pub struct Ports<'a, R, V, C, S, W> {
    pub repo: &'a R,
    pub vault: &'a V,
    pub clock: &'a C,
    pub secrets: &'a S,
    pub switch: &'a W,
    pub excluded: Vec<ParticipantId>,
}

pub(super) fn diverged(plan: &Plan, what: String) -> CliError {
    CliError(format!(
        "{} diverges from the fixture: {what}; use a fresh deal",
        plan.deal
    ))
}

fn message_id(plan: &Plan, n: usize) -> Result<MessageId, CliError> {
    MessageId::new(format!("fixture-{}-{n}", plan.deal)).map_err(CliError::new)
}

fn price_cents(offer: &Offer) -> Option<i64> {
    match &offer.terms {
        Terms::PurchaseOrder(t) => Some(t.price_per_kg().minor),
        Terms::Lease(_) => None,
    }
}

/// Returns a one-line summary; every step goes through the same use cases as live chat.
pub async fn play<R, V, C, S, W>(
    p: Ports<'_, R, V, C, S, W>,
    plan: Plan,
) -> Result<String, CliError>
where
    R: DealRepository,
    V: LimitVaultPort,
    C: Clock,
    S: SecretSource,
    W: SwitchPort,
{
    let stored = load(&p, &plan).await?;
    let prices = scripted_offers(&plan, &stored)?;
    let existing = stored.deal.offers().to_vec();
    if existing.len() > prices.len() {
        let what = format!(
            "{} offers made, the script has {}",
            existing.len(),
            prices.len()
        );
        return Err(diverged(&plan, what));
    }
    let today = i32::try_from(p.clock.now().seconds() / 86_400).unwrap_or(0);
    let mut desks = Vec::new();
    for role in [Role::A, Role::B] {
        desks.push(seal_and_bind(&p, &plan, &stored, role, today).await?);
    }
    // A resumed run keeps the delivery day of the offers already made.
    let deliver = existing
        .first()
        .and_then(|o| match &o.terms {
            Terms::PurchaseOrder(t) => Some(t.deliver_by()),
            Terms::Lease(_) => None,
        })
        .unwrap_or(Date::from_days(today + OFFER_DELIVER));
    let room = stored.deal.room().clone();
    for (n, &(role, cents)) in prices.iter().enumerate() {
        let id = message_id(&plan, n + 1)?;
        if let Some(made) = existing.get(n) {
            if made.by != role || price_cents(made) != Some(cents) || made.message_id != id {
                return Err(diverged(
                    &plan,
                    format!("offer O-{} is not the scripted one", n + 1),
                ));
            }
            continue;
        }
        let offer = submit_scripted(&p, &plan, n, (role, cents), deliver, &desks, id).await?;
        post_card(&p, &plan, &room, &offer).await?;
    }
    Ok(format!(
        "{} at round {}: {} offers ({} already there), last O-{} by the seller at {}/kg, buyer to move",
        plan.deal,
        plan.round,
        prices.len(),
        existing.len(),
        prices.len(),
        format::money(Money::new(SELLER_LAST, Currency::Usd))
    ))
}

/// The scripted (role, cents per kg) offers, checked to fit the stored deal.
fn scripted_offers(plan: &Plan, stored: &StoredDeal) -> Result<Vec<(Role, i64)>, CliError> {
    if stored.deal.kind() != DealKind::PurchaseOrder {
        return Err(CliError("the fixture scripts a purchase-order deal".into()));
    }
    let prices = scripted_prices(stored.deal.opener(), plan.round);
    if prices.last().map(|(r, _)| *r) != Some(Role::B) {
        let msg = format!("round {} leaves no seller offer to end on", plan.round);
        return Err(CliError(msg));
    }
    Ok(prices)
}

/// Makes the `n`th scripted offer (zero-based) from the role's negotiator in its desk room.
async fn submit_scripted<R, V, C, S, W>(
    p: &Ports<'_, R, V, C, S, W>,
    plan: &Plan,
    n: usize,
    (role, cents): (Role, i64),
    deliver: Date,
    desks: &[RoomId],
    id: MessageId,
) -> Result<Offer, CliError>
where
    R: DealRepository,
    V: LimitVaultPort,
    C: Clock,
{
    let i = usize::from(role == Role::B);
    let terms = PurchaseOrderTerms::new(
        Money::new(cents, Currency::Usd),
        Quantity::kg(400),
        deliver,
        45,
    )
    .map_err(CliError::new)?;
    let cmd = SubmitOffer {
        deal: plan.deal.clone(),
        sender: plan.agents[i].clone(),
        room: desks[i].clone(),
        terms: Terms::PurchaseOrder(terms),
        note: Note::default(),
        message_id: id,
    };
    let outcome = retry(async || submit_offer(p.repo, p.vault, p.clock, cmd.clone()).await)
        .await
        .map_err(|e| fail(plan, e))?;
    let OfferOutcome::Made { offer } = outcome else {
        return Err(CliError(format!("scripted offer O-{} was not made", n + 1)));
    };
    Ok(offer)
}

/// Re-runs a use case when the deal changed under it (the running notary wrote meanwhile).
/// A conflict commits nothing, so each try starts from the deal as stored.
pub(super) async fn retry<T, F: AsyncFnMut() -> Result<T, AppError>>(
    mut f: F,
) -> Result<T, AppError> {
    let mut tries = 1;
    loop {
        match f().await {
            Err(AppError::Repo(RepoError::Conflict)) if tries < 3 => tries += 1,
            done => return done,
        }
    }
}

pub(super) fn fail(plan: &Plan, e: AppError) -> CliError {
    CliError(format!("{}: {e}", plan.deal))
}

pub(super) async fn load<R: DealRepository, V, C, S, W>(
    p: &Ports<'_, R, V, C, S, W>,
    plan: &Plan,
) -> Result<StoredDeal, CliError> {
    p.repo
        .load(&plan.deal)
        .await
        .map_err(|e| fail(plan, e.into()))?
        .ok_or_else(|| CliError(format!("no deal {}; run its kickoff first", plan.deal)))
}

async fn post_card<R, V, C, S, W: SwitchPort>(
    p: &Ports<'_, R, V, C, S, W>,
    plan: &Plan,
    room: &RoomId,
    offer: &Offer,
) -> Result<(), CliError> {
    let card = render::offer_card(DealKind::PurchaseOrder, offer);
    p.switch
        .post(room, &card)
        .await
        .map_err(|e| fail(plan, e.into()))
}
