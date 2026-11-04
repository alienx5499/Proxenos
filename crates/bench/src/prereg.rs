//! Pre-registered inputs: scenarios.json, answer_key.json and the key's sha256 gate.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::BenchError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buyer,
    Seller,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Side::Buyer => Side::Seller,
            Side::Seller => Side::Buyer,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Side::Buyer => "buyer",
            Side::Seller => "seller",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScenarioFile {
    /// True only in the shipped example files.
    #[serde(default)]
    pub example: bool,
    pub seed: u64,
    pub scenarios: Vec<Scenario>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Scenario {
    pub id: String,
    /// Deal kind as the notary names it, e.g. `purchase-order`.
    pub kind: String,
    pub briefs: BTreeMap<Side, String>,
    pub injection: Option<Injection>,
}

/// The injection script: the counterparty's human addresses `target` at `at_round`,
/// and the counterparty's first offer from that round on carries `content_note`.
#[derive(Clone, Debug, Deserialize)]
pub struct Injection {
    pub target: Side,
    pub at_round: u32,
    pub human_text: String,
    pub content_note: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AnswerKey {
    #[serde(default)]
    pub example: bool,
    /// Money limits are in this currency's major units (`INR` or `USD`).
    pub currency: String,
    pub scenarios: BTreeMap<String, Answer>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Answer {
    /// None: no zone of agreement; the correct outcome is no deal.
    pub zopa: Option<Zopa>,
    /// Member of the reduced 8-scenario subset.
    #[serde(default)]
    pub subset8: bool,
    pub canaries: Option<Canaries>,
    /// Sealed limits per side, field name to constraint (same shape the seal endpoint takes).
    pub limits: BTreeMap<Side, BTreeMap<String, Constraint>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Zopa {
    pub field: String,
    /// Seller's floor.
    pub low: f64,
    /// Buyer's ceiling.
    pub high: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Canaries {
    pub human: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    AtMost(Value),
    AtLeast(Value),
    NoLaterThan(Value),
    NoEarlierThan(Value),
    Between(Value, Value),
}

impl Constraint {
    /// Values that cannot be ordered against the bound (missing, other type) fail.
    pub fn admits(&self, v: &Value) -> bool {
        let le =
            |a: &Value, b: &Value| matches!(compare(a, b), Some(Ordering::Less | Ordering::Equal));
        match self {
            Constraint::AtMost(b) | Constraint::NoLaterThan(b) => le(v, b),
            Constraint::AtLeast(b) | Constraint::NoEarlierThan(b) => le(b, v),
            Constraint::Between(lo, hi) => le(lo, v) && le(v, hi),
        }
    }

    /// The numeric bounds, for leak matching.
    pub fn numbers(&self) -> Vec<f64> {
        match self {
            Constraint::AtMost(b)
            | Constraint::AtLeast(b)
            | Constraint::NoLaterThan(b)
            | Constraint::NoEarlierThan(b) => number(b).into_iter().collect(),
            Constraint::Between(lo, hi) => number(lo).into_iter().chain(number(hi)).collect(),
        }
    }
}

/// A JSON number or a numeric string.
pub fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Numbers (plain or in a domain shape) numerically; other strings lexically.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    if let (Some(x), Some(y)) = (crate::domain_json::scalar(a), crate::domain_json::scalar(b)) {
        return x.partial_cmp(&y);
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub struct Prereg {
    pub scenarios: ScenarioFile,
    pub key: AnswerKey,
    pub key_sha: String,
    pub scenarios_sha: String,
}

impl Prereg {
    pub fn example(&self) -> bool {
        self.scenarios.example || self.key.example
    }
}

/// Loads `<dir>/scenarios.json` and `<dir>/answer_key.json`. Refuses unless the key's sha256
/// equals both `expect_key_sha` (the value cited from the pre-registration commit) and the
/// first token of `answer_key.sha256`, and the scenarios' sha256 equals `scenarios.sha256`.
pub fn load(dir: &Path, expect_key_sha: &str) -> Result<Prereg, BenchError> {
    let key_bytes = read(&dir.join("answer_key.json"))?;
    let scenario_bytes = read(&dir.join("scenarios.json"))?;
    let key_sha = sha256_hex(&key_bytes);
    let scenarios_sha = sha256_hex(&scenario_bytes);
    let expected = expect_key_sha.trim().to_lowercase();
    if expected != key_sha {
        return Err(BenchError::KeyHash {
            file: "--expect-key-sha".into(),
            pinned: expected,
            actual: key_sha,
        });
    }
    for (file, actual) in [
        ("answer_key.sha256", &key_sha),
        ("scenarios.sha256", &scenarios_sha),
    ] {
        let pinned = pinned(&dir.join(file))?;
        if &pinned != actual {
            return Err(BenchError::KeyHash {
                file: file.into(),
                pinned,
                actual: actual.clone(),
            });
        }
    }
    let scenarios: ScenarioFile = serde_json::from_slice(&scenario_bytes)?;
    let key: AnswerKey = serde_json::from_slice(&key_bytes)?;
    crate::validate::check(&scenarios, &key)?;
    Ok(Prereg {
        scenarios,
        key,
        key_sha,
        scenarios_sha,
    })
}

fn pinned(path: &Path) -> Result<String, BenchError> {
    Ok(String::from_utf8_lossy(&read(path)?)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase())
}

fn read(path: &Path) -> Result<Vec<u8>, BenchError> {
    std::fs::read(path).map_err(|e| BenchError::Input(format!("{}: {e}", path.display())))
}

#[cfg(test)]
#[path = "prereg_tests.rs"]
pub mod tests;
