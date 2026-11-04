//! Command line: `serve` (the notary itself) and the operator's tools.

pub mod admit;
pub mod fixture;
mod fixture_bind;
mod fixture_play;
#[cfg(test)]
mod fixture_tests;
pub mod register;
pub mod verify;

use std::io::Read;
use std::path::Path;

use clap::{Parser, Subcommand};

use crate::config::Secret;

#[derive(Debug, Parser)]
#[command(name = "proxenos", about = "Proxenos notary")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the notary: Switch stream, chat router and HTTP API.
    Serve,
    /// Add the deal's party rule to the notary's addressing policy, via the running notary.
    Admit(admit::AdmitArgs),
    /// Register the notary with Switch once; prints the agent id, writes the credentials file.
    Register(register::RegisterArgs),
    /// Check a receipt's signature offline.
    Verify(verify::VerifyArgs),
    /// Seed a deal at a given round with scripted limits and offers (demo set-up).
    Fixture(fixture::FixtureArgs),
}

/// A secret from a file (`-` reads stdin) or, without one, from the environment; never argv.
/// Surrounding whitespace is trimmed; empty counts as unset.
pub fn read_secret(env: &str, file: Option<&Path>) -> Result<Option<Secret>, CliError> {
    let text = match file {
        Some(p) if p == Path::new("-") => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| CliError(format!("stdin: {e}")))?;
            s
        }
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| CliError(format!("{}: {e}", p.display())))?
        }
        None => std::env::var(env).unwrap_or_default(),
    };
    let text = text.trim();
    Ok((!text.is_empty()).then(|| Secret::new(text)))
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CliError(pub String);

impl CliError {
    pub fn new(msg: impl std::fmt::Display) -> Self {
        Self(msg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn parses_each_subcommand() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["proxenos", "admit", "D-0007"]).is_err());
        let c = Cli::try_parse_from([
            "proxenos", "admit", "D-0007", "--owner", "asha", "--owner", "ravi", "--agent", "a1",
            "--agent", "a2",
        ])
        .unwrap();
        assert!(Cli::try_parse_from(["proxenos", "admit", "D-0007", "--token", "t"]).is_err());
        assert!(matches!(c.command, Command::Admit(a) if a.deal == "D-0007"));
        let c = Cli::try_parse_from([
            "proxenos",
            "register",
            "--switch-url",
            "http://s",
            "--out",
            "n.env",
        ])
        .unwrap();
        assert!(matches!(c.command, Command::Register(r) if r.name == "notary" && !r.force));
        let c = Cli::try_parse_from(["proxenos", "verify", "r.json", "--key", "ab"]).unwrap();
        assert!(matches!(c.command, Command::Verify(_)));
        let c = Cli::try_parse_from([
            "proxenos",
            "fixture",
            "D-0008",
            "--round",
            "3",
            "--negotiator",
            "buyer-agent-2",
            "--buyer-seal-file",
            "b.txt",
            "--buyer-owner",
            "u1",
            "--seller-owner",
            "u2",
            "--buyer-agent",
            "a1",
            "--seller-agent",
            "a2",
        ])
        .unwrap();
        let Command::Fixture(f) = c.command else {
            panic!()
        };
        assert_eq!(f.round, 3);
        assert_eq!(f.buyer_negotiator(), "buyer-agent-2");
    }
}
