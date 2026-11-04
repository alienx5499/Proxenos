//! `proxenos serve`: builds the ports from config, runs the one router task and the HTTP API.

mod inputs;
mod ports;
mod startup;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use proxenos_domain::party::ParticipantId;
use proxenos_switch_client::stream::{Item, StreamSpec, Subscription};
use proxenos_switch_client::{Credentials, SwitchClient};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use self::inputs::{Input, drive};
pub use self::ports::{NotaryPorts, Parts};
use self::startup::{check_vault, load_signer, shown};
use crate::adapters::SystemClock;
use crate::adapters::switch::SwitchAdapter;
use crate::chat::router::{Router, RouterConfig};
use crate::config::Config;
use crate::crypto::secrets::OsSecrets;
use crate::http::routes::router;
use crate::http::{AppState, HttpConfig};
use crate::store::sqlite::Store;
use crate::switch_admin::GatewayAdmin;
use crate::vault::aes::{AesVault, VaultKey};

/// Inputs queued for the router task; producers wait when it is full.
const INPUT_QUEUE: usize = 256;
/// One-shot room events waiting to be merged (the adapter drops, never blocks, beyond this).
const ROOM_QUEUE: usize = 256;

/// Everything `run` needs, built and checked before anything listens.
pub struct Wired {
    pub ports: NotaryPorts,
    pub client: SwitchClient,
    pub room_events: mpsc::Receiver<Item>,
    pub router_cfg: RouterConfig,
    pub http_cfg: HttpConfig,
}

fn id(value: &str, what: &str) -> Result<ParticipantId, String> {
    ParticipantId::new(value).map_err(|e| format!("{what}: {e}"))
}

pub async fn build(cfg: &Config) -> Result<Wired, String> {
    let vault_key = VaultKey::parse(Some(cfg.vault_key.expose())).map_err(|e| e.to_string())?;
    let secrets = OsSecrets::parse(Some(cfg.secret_key.expose())).map_err(|e| e.to_string())?;
    let store = Store::open(Path::new(&cfg.db))
        .await
        .map_err(|e| format!("database {}: {e}", shown(&cfg.db)))?;
    let signer = load_signer(cfg, &store).await?;
    let vault = AesVault::new(&vault_key);
    check_vault(cfg, &store, &vault).await?;
    let creds = Credentials {
        agent_id: cfg.agent_id.clone(),
        api_key: cfg.agent_key.expose().to_owned(),
    };
    let client = SwitchClient::new(&cfg.switch_url, creds).map_err(|e| e.to_string())?;
    let (room_tx, room_events) = mpsc::channel(ROOM_QUEUE);
    let switch = SwitchAdapter::new(client.clone(), Some(room_tx));
    let admin = GatewayAdmin::new(
        &cfg.gateway_url,
        cfg.gateway_key.expose(),
        &cfg.agent_id,
        &cfg.bridge_id,
        cfg.allow_http_gateway,
    )
    .map_err(|e| e.0)?;
    warn_risky_urls(cfg);
    let (router_cfg, http_cfg) = chat_and_http_configs(cfg)?;
    let parts = Parts {
        store,
        vault,
        clock: SystemClock,
        secrets,
        signer,
        switch,
        admin,
    };
    Ok(Wired {
        ports: NotaryPorts(Arc::new(parts)),
        client,
        room_events,
        router_cfg,
        http_cfg,
    })
}

fn warn_risky_urls(cfg: &Config) {
    if cfg.allow_http_gateway && cfg.gateway_url.starts_with("http://") {
        eprintln!(
            "warning: PROXENOS_ALLOW_HTTP_GATEWAY is set; the operator Gateway key travels \
             unencrypted to {}",
            cfg.gateway_url
        );
    }
    if cfg.public_url.contains("://0.0.0.0") {
        eprintln!(
            "warning: seal links start with {}, which a browser cannot open; \
             set PROXENOS_PUBLIC_URL",
            cfg.public_url
        );
    }
}

fn chat_and_http_configs(cfg: &Config) -> Result<(RouterConfig, HttpConfig), String> {
    let notary = id(&cfg.notary_sender, "PROXENOS_NOTARY_SENDER_ID")?;
    let relays = cfg
        .relay_ids
        .iter()
        .map(|r| id(r, "PROXENOS_RELAY_IDS"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut excluded = vec![notary.clone()];
    excluded.extend(relays.iter().cloned());
    let router_cfg = RouterConfig {
        notary,
        notary_name: cfg.notary_name.clone(),
        operator: cfg.operator_id.clone(),
        relays,
        sign_cards: cfg.sign_cards,
        deal_secs: cfg.deal_secs,
        max_rounds: cfg.max_rounds,
        public_url: cfg.public_url.clone(),
    };
    let http_cfg = HttpConfig {
        operator_token: cfg.operator_token.expose().to_owned(),
        public_url: cfg.public_url.clone(),
        excluded,
        deal_secs: cfg.deal_secs,
        max_rounds: cfg.max_rounds,
    };
    Ok((router_cfg, http_cfg))
}

fn every(tx: &mpsc::Sender<Input>, period: Duration, input: fn() -> Input) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if tx.send(input()).await.is_err() {
                return;
            }
        }
    });
}

/// Forwards `rx` into the router queue until either side closes.
fn forward<T: Send + 'static>(
    mut rx: mpsc::Receiver<T>,
    tx: &mpsc::Sender<Input>,
    wrap: fn(T) -> Input,
) -> JoinHandle<()> {
    let tx = tx.clone();
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            if tx.send(wrap(item)).await.is_err() {
                return;
            }
        }
    })
}

/// Forwards the `scope=all` stream into the router queue, then says it ended.
fn feed_stream(mut stream: Subscription, tx: &mpsc::Sender<Input>) -> JoinHandle<()> {
    let tx = tx.clone();
    tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            if tx.send(Input::Stream(item)).await.is_err() {
                return;
            }
        }
        let _ = tx.send(Input::StreamEnded).await;
    })
}

/// Runs until `shutdown` resolves or the router stops on a fatal error (returned).
/// Seal links of deals opened by kickoff go to `links`, never to Switch.
pub async fn run(
    w: Wired,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    links: mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel(INPUT_QUEUE);
    let (stop_tx, stop_rx) = watch::channel(false);
    let stream = w.client.subscribe(StreamSpec::all_addressed());
    let feeders = [
        feed_stream(stream, &tx),
        forward(w.room_events, &tx, Input::Room),
    ];
    let (opened_tx, opened) = mpsc::channel(INPUT_QUEUE);
    let wake_feeder = forward(opened, &tx, Input::WakeTurn);
    every(&tx, Duration::from_secs(2), || Input::CardTick);
    every(&tx, Duration::from_secs(30), || Input::ExpireTick);
    every(&tx, Duration::from_secs(3_600), || Input::PruneTick);
    drop(tx);

    let public_url = w.http_cfg.public_url.clone();
    let sign_cards = w.router_cfg.sign_cards;
    let state = AppState::new(w.ports.clone(), w.http_cfg).notify_opened(opened_tx);
    let app = router(Arc::new(state));
    let chat = Router::new(w.ports, w.router_cfg);
    let router_task = {
        let (stop_tx, stop_rx) = (stop_tx.clone(), stop_rx.clone());
        tokio::spawn(async move {
            let result = drive(chat, rx, stop_rx, sign_cards, &public_url, &links).await;
            let _ = stop_tx.send(true);
            result
        })
    };
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop_tx.send(true);
    });
    let mut http_stop = stop_rx;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = http_stop.wait_for(|stop| *stop).await;
        })
        .await
        .map_err(|e| format!("http: {e}"))?;
    let result = router_task.await.map_err(|e| format!("router task: {e}"))?;
    for f in feeders {
        f.abort();
    }
    wake_feeder.abort();
    result
}

/// `proxenos serve`: config from the environment, seal links printed to stdout.
pub async fn serve(cfg: Config) -> Result<(), String> {
    let wired = build(&cfg).await?;
    let listener = TcpListener::bind(cfg.addr)
        .await
        .map_err(|e| format!("listen on {}: {e}", cfg.addr))?;
    eprintln!(
        "proxenos notary on {} as agent {} (sender id {}); database {}, receipt key {}",
        cfg.addr,
        cfg.agent_id,
        cfg.notary_sender,
        shown(&cfg.db),
        if cfg.receipt_key.is_some() {
            "from PROXENOS_RECEIPT_KEY".to_owned()
        } else {
            shown(&cfg.receipt_key_file)
        }
    );
    let (links_tx, mut links_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(line) = links_rx.recv().await {
            println!("{line}");
        }
    });
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("shutting down");
    };
    run(wired, listener, ctrl_c, links_tx).await
}
