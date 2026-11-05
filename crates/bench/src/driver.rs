//! Runs scenarios per arm, writes bench/results, --resume skips finished runs.

use std::path::Path;
use std::time::Duration;

use proxenos_switch_client::SwitchClient;

use crate::BenchError;
use crate::agent_loop::Seat;
use crate::cli::CommandCli;
use crate::config::{Live, connect, need};
use crate::extract::Grammar;
use crate::gateway::{fresh_rooms, remove_rooms, set_policies};
use crate::ledger::Ledger;
use crate::listen::listen;
use crate::notary_api::open_and_seal;
use crate::prereg::{Answer, Prereg, Scenario, Side};
use crate::record::{self, RunRecord};
use crate::run::{Limits, Referee, Run};
use crate::{domain_json, prompt};

/// Caps. Turns: 12 rounds x 2 sides x (3 probes + 1 move); raised from 30, which arms A and
/// B spent by round 6 on probes. No arm D run used more than 26.
pub const TURN_BUDGET: u32 = 96;
pub const MAX_ROUNDS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Arm {
    #[value(name = "A")]
    A,
    #[value(name = "B")]
    B,
    #[value(name = "D")]
    D,
}

impl Arm {
    pub fn name(self) -> &'static str {
        match self {
            Arm::A => "A",
            Arm::B => "B",
            Arm::D => "D",
        }
    }

    /// Feeds the deal id, so each arm and scenario opens its own deal.
    fn index(self) -> u32 {
        match self {
            Arm::A => 1,
            Arm::B => 2,
            Arm::D => 4,
        }
    }
}

/// One benchmark invocation.
pub struct Plan<'a> {
    pub arms: &'a [Arm],
    pub prereg: &'a Prereg,
    pub results: &'a Path,
    pub resume: bool,
    pub subset8: bool,
    pub live: &'a Live,
    pub guide: &'a str,
}

/// Runs every (arm, scenario) in order. Without `resume`, any existing result is an error
/// before the first run starts; with it, finished runs are skipped once their hashes match.
pub async fn run_arms(plan: &Plan<'_>) -> Result<(), BenchError> {
    let pre = plan.prereg;
    let todo: Vec<(Arm, u32, &Scenario)> = plan
        .arms
        .iter()
        .flat_map(|&arm| {
            pre.scenarios
                .scenarios
                .iter()
                .enumerate()
                .map(move |(i, sc)| (arm, i as u32 + 1, sc))
        })
        .filter(|(_, _, sc)| !plan.subset8 || pre.key.scenarios[&sc.id].subset8)
        .collect();
    let path = |arm: Arm, sc: &Scenario| record::path(plan.results, arm.name(), &sc.id);
    if !plan.resume {
        let existing: Vec<String> = todo
            .iter()
            .map(|(arm, _, sc)| path(*arm, sc))
            .filter(|p| p.exists())
            .map(|p| p.display().to_string())
            .collect();
        if !existing.is_empty() {
            return Err(BenchError::Input(format!(
                "results exist ({}); pass --resume to skip finished runs",
                existing.join(", ")
            )));
        }
    }
    for (arm, number, sc) in todo {
        let p = path(arm, sc);
        if p.exists() {
            record::check_hashes(&record::read_one(&p)?, &pre.key_sha, &pre.scenarios_sha)?;
            eprintln!("skip {} {} (finished)", arm.name(), sc.id);
            continue;
        }
        let attempt = record::next_attempt(plan.results, arm.name(), &sc.id)?;
        eprintln!("run {} {} (attempt {attempt})", arm.name(), sc.id);
        let rec = run_one(
            plan,
            arm,
            deal_number(plan.live.deal_base, arm, attempt, number)?,
            sc,
        )
        .await?;
        record::write(plan.results, &rec)?;
    }
    Ok(())
}

/// base + arm*1000 + attempt*100 + scenario number, kept to the four digits of `D-dddd`.
fn deal_number(base: u32, arm: Arm, attempt: u32, number: u32) -> Result<u32, BenchError> {
    let n = base + arm.index() * 1000 + attempt * 100 + number;
    if attempt > 9 || number > 99 || n > 9999 {
        return Err(BenchError::Input(format!(
            "no free deal id (attempt {attempt}, scenario {number}, base {base}); lower --deal-base or clear .attempt files"
        )));
    }
    Ok(n)
}

/// Each run gets its own rooms, removed afterwards even when the run fails.
async fn run_one(
    plan: &Plan<'_>,
    arm: Arm,
    deal_number: u32,
    sc: &Scenario,
) -> Result<RunRecord, BenchError> {
    let deal_id = format!("D-{deal_number:04}");
    let (live, rooms) = fresh_rooms(plan.live, &deal_id, arm, sc).await?;
    let pointed = Plan {
        live: &live,
        ..*plan
    };
    let rec = run_in(&pointed, arm, deal_number, sc).await;
    if let Err(e) = remove_rooms(&live, &rooms).await {
        eprintln!("{deal_id}: rooms not removed: {e}");
    }
    rec
}

async fn run_in(
    plan: &Plan<'_>,
    arm: Arm,
    deal_number: u32,
    sc: &Scenario,
) -> Result<RunRecord, BenchError> {
    let live = plan.live;
    let ans: &Answer = &plan.prereg.key.scenarios[&sc.id];
    let deal_room = need(&live.deal_room, "BENCH_DEAL_ROOM")?.to_owned();
    set_policies(live, arm).await?;
    let buyer = seat(plan, arm, sc, ans, &deal_room, Side::Buyer)?;
    let seller = seat(plan, arm, sc, ans, &deal_room, Side::Seller)?;
    // Listen first: the notary posts its first wake as soon as the deal opens.
    let subs = listen([&buyer.client, &seller.client], Duration::from_secs(30)).await?;
    let (referee, deal_id) = referee(plan, arm, deal_number, sc, ans, &deal_room).await?;
    let injector = injector(live)?;
    let run = Run {
        buyer,
        seller,
        referee,
        injector,
        names: [live.buyer_name.clone(), live.seller_name.clone()],
        injection: sc.injection.clone(),
        limits: Limits {
            turns: TURN_BUDGET,
            rounds: MAX_ROUNDS,
            // A healthy turn posts within a minute; 3 quiet minutes means the run is stuck.
            idle: Duration::from_secs(180),
            poll: Duration::from_secs(5),
        },
    };
    let rec = RunRecord {
        example: plan.prereg.example(),
        key_sha: plan.prereg.key_sha.clone(),
        scenarios_sha: plan.prereg.scenarios_sha.clone(),
        deal_id,
        deal_room: deal_room.clone(),
        ..RunRecord::blank(arm.name(), &sc.id)
    };
    run.execute(rec, subs).await
}

/// Arm D referees with the local ledger; A and B open and seal a notary deal.
async fn referee(
    plan: &Plan<'_>,
    arm: Arm,
    deal_number: u32,
    sc: &Scenario,
    ans: &Answer,
    deal_room: &str,
) -> Result<(Referee, Option<String>), BenchError> {
    Ok(match arm {
        Arm::D => (Referee::Ledger(Ledger::new(MAX_ROUNDS, &sc.kind)), None),
        Arm::A | Arm::B => {
            let deal_id = format!("D-{deal_number:04}");
            let api = open_and_seal(
                plan.live,
                &deal_id,
                deal_room,
                sc,
                ans,
                &plan.prereg.key.currency,
            )
            .await?;
            (
                Referee::Notary {
                    api,
                    deal_id: deal_id.clone(),
                },
                Some(deal_id),
            )
        }
    })
}

fn injector(live: &Live) -> Result<Option<SwitchClient>, BenchError> {
    Ok(match (&live.injector_agent_id, &live.injector_agent_key) {
        (Some(id), Some(key)) => Some(connect(need(&live.switch_url, "SWITCH_URL")?, id, key)?),
        _ => None,
    })
}

fn seat(
    plan: &Plan<'_>,
    arm: Arm,
    sc: &Scenario,
    ans: &Answer,
    deal_room: &str,
    side: Side,
) -> Result<Seat<CommandCli>, BenchError> {
    let live = plan.live;
    let brief = sc
        .briefs
        .get(&side)
        .ok_or_else(|| BenchError::Input(format!("{} has no {side} brief", sc.id)))?;
    let (grammar, address, limits) = match arm {
        Arm::D => (
            Grammar::Direct,
            live.name(side.other()).to_owned(),
            Some(direct_limits(plan, sc, ans, side)?),
        ),
        Arm::A | Arm::B => (Grammar::Notary, "notary".to_owned(), None),
    };
    Ok(Seat {
        side,
        client: live.client(side)?,
        cli: live.cli(side),
        grammar,
        standing: prompt::standing(
            side,
            &sc.kind,
            plan.guide,
            brief,
            grammar,
            limits.as_deref(),
        ),
        address,
        deal_room: deal_room.to_owned(),
        desk_room: live
            .by_side(side, &live.buyer_desk, &live.seller_desk)
            .clone(),
        notary_sender: match arm {
            Arm::D => None,
            Arm::A | Arm::B => {
                Some(need(&live.notary_sender, "PROXENOS_NOTARY_SENDER_ID")?.to_owned())
            }
        },
        last_state: None,
        last_sent: None,
        round_log: Default::default(),
    })
}

/// Arm D has no notary, so each agent reads its own limits in the standing prompt.
fn direct_limits(
    plan: &Plan<'_>,
    sc: &Scenario,
    ans: &Answer,
    side: Side,
) -> Result<String, BenchError> {
    match ans.limits.get(&side) {
        Some(l) => Ok(serde_json::to_string_pretty(
            &domain_json::limits_for_agent(l, &plan.prereg.key.currency)?,
        )?),
        None => Err(BenchError::Input(format!("{} has no {side} limits", sc.id))),
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
