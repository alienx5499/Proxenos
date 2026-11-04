//! Proxenos benchmark driver.

mod agent_loop;
mod cli;
mod config;
mod domain_json;
mod driver;
mod extract;
mod gateway;
mod inject;
mod leak;
mod ledger;
mod listen;
mod notary_api;
mod prereg;
mod prompt;
mod record;
mod rescore;
mod results_md;
mod run;
mod validate;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::agent_loop::Seat;
use crate::config::Live;
use crate::driver::{Arm, TURN_BUDGET};
use crate::extract::Grammar;
use crate::prereg::Side;

#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error(
        "pre-registration hash mismatch: {file} pins {pinned}, the file hashes to {actual}; refusing to run"
    )]
    KeyHash {
        file: String,
        pinned: String,
        actual: String,
    },
    #[error("{0}")]
    Input(String),
    #[error("bad json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("switch: {0}")]
    Switch(#[from] proxenos_switch_client::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("cli: {0}")]
    Cli(String),
}

#[derive(Parser, Debug)]
#[command(about = "Proxenos benchmark: run arms, rescore results, or play one side live")]
#[command(group = clap::ArgGroup::new("mode").required(true).args(["arm", "rescore", "play"]))]
struct Args {
    /// Arms to run, in order (needs the live stack).
    #[arg(long, value_enum, value_delimiter = ',')]
    arm: Vec<Arm>,
    /// Skip runs that already have a result file.
    #[arg(long)]
    resume: bool,
    /// Run only the reduced subset marked `subset8` in the answer key.
    #[arg(long)]
    subset8: bool,
    /// Score a results directory offline and write RESULTS.md into it.
    #[arg(long)]
    rescore: Option<PathBuf>,
    /// Run one side live on the deal room given by --deal-room.
    #[arg(long, value_enum, requires = "deal_room")]
    play: Option<Side>,
    /// Brief file for --play.
    #[arg(long)]
    brief: Option<PathBuf>,
    /// Deal kind for --play, which picks the prompt's terms example.
    #[arg(long, default_value = "purchase-order")]
    kind: String,
    /// sha256 of answer_key.json as cited from the pre-registration commit.
    #[arg(long, required = true)]
    expect_key_sha: String,
    /// Permit live runs on the shipped example files (records are stamped `example`).
    #[arg(long)]
    allow_example: bool,
    #[arg(long, default_value = "bench/prereg")]
    prereg: PathBuf,
    #[arg(long, default_value = "bench/results")]
    results: PathBuf,
    #[arg(long, default_value = "docs/AGENT.md")]
    guide: PathBuf,
    #[command(flatten)]
    live: Live,
}

impl clap::ValueEnum for Side {
    fn value_variants<'a>() -> &'a [Self] {
        &[Side::Buyer, Side::Seller]
    }
    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_text(path: &PathBuf) -> Result<String, BenchError> {
    std::fs::read_to_string(path).map_err(|e| BenchError::Input(format!("{}: {e}", path.display())))
}

async fn run(args: Args) -> Result<(), BenchError> {
    let pre = prereg::load(&args.prereg, &args.expect_key_sha)?;
    if let Some(dir) = &args.rescore {
        let table = rescore::rescore(dir, &pre)?;
        record::write_atomic(&dir.join("RESULTS.md"), table.as_bytes())?;
        println!("{table}");
        return Ok(());
    }
    if pre.example() && !args.allow_example {
        return Err(BenchError::Input(
            "these are the example pre-registration files; live runs need --allow-example".into(),
        ));
    }
    let guide = read_text(&args.guide)?;
    if let Some(side) = args.play {
        let brief = args
            .brief
            .as_ref()
            .map(read_text)
            .transpose()?
            .unwrap_or_default();
        let live = &args.live;
        let mut seat = Seat {
            side,
            client: live.client(side)?,
            cli: live.cli(side),
            grammar: Grammar::Notary,
            standing: prompt::standing(side, &args.kind, &guide, &brief, Grammar::Notary, None),
            address: "notary".to_owned(),
            deal_room: config::need(&live.deal_room, "--deal-room")?.to_owned(),
            desk_room: live
                .by_side(side, &live.buyer_desk, &live.seller_desk)
                .clone(),
            notary_sender: Some(
                config::need(&live.notary_sender, "PROXENOS_NOTARY_SENDER_ID")?.to_owned(),
            ),
            last_state: None,
            last_sent: None,
            round_log: Default::default(),
        };
        return agent_loop::play(&mut seat, TURN_BUDGET).await;
    }
    if pre.example() {
        eprintln!("warning: example pre-registration files; these runs are not evidence");
    }
    eprintln!(
        "scenarios: {} (seed {})",
        pre.scenarios.scenarios.len(),
        pre.scenarios.seed
    );
    let plan = driver::Plan {
        arms: &args.arm,
        prereg: &pre,
        results: &args.results,
        resume: args.resume,
        subset8: args.subset8,
        live: &args.live,
        guide: &guide,
    };
    driver::run_arms(&plan).await
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        super::Args::command().debug_assert();
    }
}
