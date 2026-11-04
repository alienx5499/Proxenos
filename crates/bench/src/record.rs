//! One finished run as written to bench/results/<arm>/<scenario>.json.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::BenchError;
use crate::prereg::Side;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Run on the shipped example files (`--allow-example`), never evidence.
    #[serde(default)]
    pub example: bool,
    pub key_sha: String,
    pub scenarios_sha: String,
    pub arm: String,
    pub scenario: String,
    pub deal_id: Option<String>,
    pub deal_room: String,
    pub outcome: Outcome,
    pub turns: u32,
    pub rounds: u32,
    /// Every event Switch delivered to either agent, with its `addressed` flag.
    pub received: Vec<Received>,
    /// Every message the driver posted for an agent (verbatim) or the injector.
    pub posted: Vec<Posted>,
    pub injection: InjectionLog,
    /// Stream errors, gaps and subscription changes seen during the run.
    #[serde(default)]
    pub stream_notes: Vec<String>,
}

impl RunRecord {
    /// A run not yet started: no hashes, no events, outcome `running`.
    pub fn blank(arm: &str, scenario: &str) -> Self {
        Self {
            example: false,
            key_sha: String::new(),
            scenarios_sha: String::new(),
            arm: arm.to_owned(),
            scenario: scenario.to_owned(),
            deal_id: None,
            deal_room: String::new(),
            outcome: Outcome::NoDeal {
                reason: "running".into(),
            },
            turns: 0,
            rounds: 0,
            received: Vec::new(),
            posted: Vec::new(),
            injection: InjectionLog::default(),
            stream_notes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Terms agreed (notary hash issued, or an accepted offer in arm D).
    Deal { terms: Value },
    /// `reason`: expired, withdrawn, round_cap, turn_budget.
    NoDeal { reason: String },
    /// The run broke, not the negotiation: idle, stream_error, cli_failure.
    Infra { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Received {
    pub side: Side,
    pub room: String,
    pub sender: String,
    pub addressed: bool,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Posted {
    /// None: posted by the injector, or a driver prompt (`by_driver`).
    pub side: Option<Side>,
    /// A prompt the driver gave an agent directly (arm D opener); never sent to Switch.
    #[serde(default)]
    pub by_driver: bool,
    pub room: String,
    pub content: String,
    /// The CLI's full reply for this turn (empty for the injector).
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub cli_error: Option<String>,
    /// The note was replaced by the scenario's content injection.
    #[serde(default)]
    pub note_injected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InjectionLog {
    pub target: Option<Side>,
    pub human_posted: bool,
    pub note_injected: bool,
}

pub fn path(results: &Path, arm: &str, scenario: &str) -> PathBuf {
    results.join(arm).join(format!("{scenario}.json"))
}

/// Written whole through a temp file, so a crash never leaves a half record that resume would skip.
pub fn write(results: &Path, rec: &RunRecord) -> Result<(), BenchError> {
    write_atomic(
        &path(results, &rec.arm, &rec.scenario),
        &serde_json::to_vec_pretty(rec)?,
    )
}

/// Temp file, fsync, rename.
pub fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), BenchError> {
    let io = |e: std::io::Error| BenchError::Input(format!("{}: {e}", p.display()));
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = p.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp).map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)?;
    std::fs::rename(&tmp, p).map_err(io)
}

pub fn read_one(p: &Path) -> Result<RunRecord, BenchError> {
    let bytes = std::fs::read(p).map_err(|e| BenchError::Input(format!("{}: {e}", p.display())))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Refuses a record made from other pre-registration files.
pub fn check_hashes(rec: &RunRecord, key_sha: &str, scenarios_sha: &str) -> Result<(), BenchError> {
    if rec.key_sha != key_sha || rec.scenarios_sha != scenarios_sha {
        return Err(BenchError::Input(format!(
            "{} {} was recorded with other pre-registration files (key {}, scenarios {})",
            rec.arm, rec.scenario, rec.key_sha, rec.scenarios_sha
        )));
    }
    Ok(())
}

/// Counts starts of one run (kept in `<scenario>.attempt`), so a rerun after a crash
/// opens a fresh deal id. Returns this start's number, from 0.
pub fn next_attempt(results: &Path, arm: &str, scenario: &str) -> Result<u32, BenchError> {
    let p = results.join(arm).join(format!("{scenario}.attempt"));
    let n = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    write_atomic(&p, (n + 1).to_string().as_bytes())?;
    Ok(n)
}

/// Every record under `results/<arm>/`, sorted by arm then scenario.
pub fn read_all(results: &Path) -> Result<Vec<RunRecord>, BenchError> {
    let mut out = Vec::new();
    let dirs = std::fs::read_dir(results)
        .map_err(|e| BenchError::Input(format!("{}: {e}", results.display())))?;
    for dir in dirs.flatten().filter(|d| d.path().is_dir()) {
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "json") {
                let bytes = std::fs::read(&p)
                    .map_err(|e| BenchError::Input(format!("{}: {e}", p.display())))?;
                out.push(serde_json::from_slice(&bytes)?);
            }
        }
    }
    out.sort_by(|a: &RunRecord, b| (&a.arm, &a.scenario).cmp(&(&b.arm, &b.scenario)));
    Ok(out)
}
