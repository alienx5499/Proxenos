//! The router task: every input, from the stream, one-shot rooms, HTTP and timers, in order.

use proxenos_app::commands::expire_due::expire_due;
use proxenos_app::ports::{Clock, DealReader, SwitchPort};
use proxenos_app::queries::label;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::Role;
use proxenos_domain::timestamp::Timestamp;
use proxenos_switch_client::Error as SwitchClientError;
use proxenos_switch_client::stream::Item;
use tokio::sync::{mpsc, watch};

use super::NotaryPorts;
use crate::chat::inbound::Inbound;
use crate::chat::router::{Dropped, Ports, Routed, Router};

/// Message ids are remembered this long for redelivery dedupe.
const SEEN_KEEP_SECS: i64 = 7 * 86_400;

pub enum Input {
    Stream(Result<Item, SwitchClientError>),
    /// The `scope=all` subscription ended for good.
    StreamEnded,
    /// An event from a one-shot room connection, which held the room meanwhile.
    Room(Item),
    /// Both sides sealed over HTTP; wake whoever moves first.
    WakeTurn(DealId),
    CardTick,
    ExpireTick,
    PruneTick,
}

/// Returns on `stop`, or with an error the process cannot continue past.
pub async fn drive(
    router: Router<NotaryPorts>,
    mut inputs: mpsc::Receiver<Input>,
    mut stop: watch::Receiver<bool>,
    sign_cards: bool,
    public_url: &str,
    links: &mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    loop {
        let input = tokio::select! {
            _ = stop.wait_for(|s| *s) => return Ok(()),
            input = inputs.recv() => match input {
                Some(i) => i,
                None => return Ok(()),
            },
        };
        match input {
            Input::Stream(Ok(item)) | Input::Room(item) => {
                on_item(&router, item, public_url, links).await;
            }
            Input::Stream(Err(SwitchClientError::TakenOver)) => {
                return Err("another process took over the notary's Switch connection; \
                            run one notary per agent key"
                    .into());
            }
            Input::Stream(Err(e)) => eprintln!("switch stream: {e}"),
            Input::StreamEnded => {
                return Err(
                    "the Switch event stream ended (open refused?); check SWITCH_URL \
                            and the agent key"
                        .into(),
                );
            }
            Input::WakeTurn(deal) => {
                if let Err(e) = router.wake_turn(&deal).await {
                    eprintln!("turn wake for {deal} failed: {e}");
                }
            }
            Input::CardTick => {
                let r = if sign_cards {
                    router.process_card_outcomes().await
                } else {
                    router.retry_dirty_sheets().await
                };
                if let Err(e) = r {
                    eprintln!("card and sheet poll: {e}");
                }
            }
            Input::ExpireTick => expire(&router).await,
            Input::PruneTick => {
                let p = router.ports();
                let cutoff = Timestamp(Ports::clock(p).now().0 - SEEN_KEEP_SECS);
                if let Err(e) = p.0.store.prune_seen(cutoff).await {
                    eprintln!("pruning seen messages: {e}");
                }
            }
        }
    }
}

async fn on_item(
    router: &Router<NotaryPorts>,
    item: Item,
    public_url: &str,
    links: &mpsc::UnboundedSender<String>,
) {
    match item {
        Item::Event(event) => {
            let Some(msg) = Inbound::from_event(&event) else {
                return;
            };
            match router.handle(msg).await {
                Ok(Routed::Opened { deal, seal_tokens }) => {
                    let kind = match Ports::repo(router.ports()).load(&deal).await {
                        Ok(Some(s)) => s.deal.kind(),
                        _ => return eprintln!("{deal} opened but could not be reloaded"),
                    };
                    for (role, token) in
                        [(Role::A, seal_tokens.role_a), (Role::B, seal_tokens.role_b)]
                    {
                        let side = label(kind, role);
                        let _ = links.send(format!(
                            "SEAL LINK {deal} {side}: {public_url}/seal/{token}"
                        ));
                    }
                }
                // Duplicates, own posts, unaddressed messages and canned texts are routine.
                Ok(Routed::Dropped(
                    d @ (Dropped::Platform
                    | Dropped::UnboundAgentText
                    | Dropped::KickoffNotFromOperator
                    | Dropped::NoDeal
                    | Dropped::RateLimited),
                )) => eprintln!("dropped a message in {}: {d:?}", event.room_id),
                Ok(_) => {}
                Err(e) => eprintln!("router: {e}"),
            }
        }
        Item::Gap(g) => eprintln!(
            "switch stream gap ({}): events may be missing in {:?}",
            g.reason, g.rooms
        ),
        Item::SubscriptionChanged(c) => {
            eprintln!("switch rooms changed ({}): {:?}", c.reason, c.rooms)
        }
        Item::Connected(c) => eprintln!("switch stream connected: {} rooms", c.rooms.len()),
    }
}

/// Deals past their deadline are closed; the deal room is told.
async fn expire(router: &Router<NotaryPorts>) {
    let p = router.ports();
    let report = match expire_due(Ports::repo(p), Ports::clock(p)).await {
        Ok(r) => r,
        Err(e) => return eprintln!("expiry sweep: {e}"),
    };
    for (deal, e) in report.failed {
        eprintln!("expiry of {deal}: {e}");
    }
    for deal in report.expired {
        let Ok(Some(stored)) = Ports::repo(p).load(&deal).await else {
            continue;
        };
        let text = format!("{deal} expired: deadline passed");
        if let Err(e) = Ports::switch(p).post(stored.deal.room(), &text).await {
            eprintln!("posting expiry of {deal}: {e}");
        }
    }
}
