//! The subject CLI (`claude -p` or `codex exec`), run sandboxed: empty working directory,
//! cleared environment, tools off where the CLI allows it, own process group.

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::process::Command;

use crate::BenchError;

/// One headless CLI call: prompt in, final reply text out.
pub trait AgentCli {
    fn run(&self, prompt: &str) -> impl Future<Output = Result<String, BenchError>> + Send;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum CliKind {
    #[default]
    Claude,
    Codex,
}

/// Inherited by every CLI: enough to find the binary and its login under HOME.
const BASE_ENV: &[&str] = &["PATH", "HOME", "LANG", "TMPDIR"];
/// Each CLI's own auth variables, nothing else. Claude's keychain login is looked up by USER.
const CLAUDE_ENV: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "USER"];
const CODEX_ENV: &[&str] = &["OPENAI_API_KEY", "CODEX_HOME"];

impl CliKind {
    fn program(self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
        }
    }

    /// Claude: print mode, no built-in tools (`--tools ""`), no MCP servers beyond an empty
    /// `--mcp-config` set (`--strict-mcp-config`), no saved session, and no settings files
    /// (`--setting-sources ""`): the operator's skills, CLAUDE.md and hooks under HOME stay
    /// out; the login still works.
    /// Codex: read-only sandbox, no git check (the working directory is an empty temp dir).
    /// Codex's read-only sandbox can still read files outside the working directory, so run
    /// the bench as an OS user that cannot read the pre-registration files' originals.
    fn args(self, prompt: &str) -> Vec<String> {
        let fixed: &[&str] = match self {
            CliKind::Claude => &[
                "-p",
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--setting-sources",
                "",
            ],
            CliKind::Codex => &["exec", "--sandbox", "read-only", "--skip-git-repo-check"],
        };
        fixed
            .iter()
            .map(|s| (*s).to_owned())
            .chain([prompt.to_owned()])
            .collect()
    }

    fn env(self) -> impl Iterator<Item = &'static str> {
        let own = match self {
            CliKind::Claude => CLAUDE_ENV,
            CliKind::Codex => CODEX_ENV,
        };
        BASE_ENV.iter().chain(own).copied()
    }
}

pub struct CommandCli {
    pub kind: CliKind,
    pub timeout: Duration,
}

static CALLS: AtomicU64 = AtomicU64::new(0);

/// A fresh empty directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, BenchError> {
        let n = CALLS.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("proxenos-bench-cli-{}-{n}", std::process::id()));
        std::fs::create_dir(&dir)
            .map_err(|e| BenchError::Cli(format!("{}: {e}", dir.display())))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl AgentCli for CommandCli {
    async fn run(&self, prompt: &str) -> Result<String, BenchError> {
        let scratch = Scratch::new()?;
        let mut cmd = Command::new(self.kind.program());
        cmd.args(self.kind.args(prompt))
            .current_dir(&scratch.0)
            .env_clear()
            .envs(
                self.kind
                    .env()
                    .filter_map(|k| std::env::var_os(k).map(|v| (k, v))),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| BenchError::Cli(format!("{} did not start: {e}", self.kind.program())))?;
        let pgid = child.id();
        let out = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Ok(out) => out.map_err(|e| BenchError::Cli(e.to_string()))?,
            Err(_) => {
                kill_group(pgid);
                return Err(BenchError::Cli(format!(
                    "no reply within {:?}",
                    self.timeout
                )));
            }
        };
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(BenchError::Cli(format!("{}: {}", out.status, err.trim())));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The child leads its own group (`process_group(0)`), so its pid is the group id.
fn kill_group(pgid: Option<u32>) {
    if let Some(id) = pgid {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{id}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_off_and_env_allowlisted() {
        let a = CliKind::Claude.args("P");
        assert_eq!(a.last().unwrap(), "P");
        for flag in ["--tools", "--setting-sources"] {
            assert!(
                a.windows(2).any(|w| w[0] == flag && w[1].is_empty()),
                "{flag}"
            );
        }
        assert!(a.contains(&"--strict-mcp-config".to_owned()));
        assert!(CliKind::Codex.args("P").contains(&"read-only".to_owned()));
        let env: Vec<_> = CliKind::Claude.env().collect();
        assert!(env.contains(&"ANTHROPIC_API_KEY") && !env.contains(&"OPENAI_API_KEY"));
        assert!(
            !env.iter()
                .any(|k| k.contains("SWITCH") || k.contains("PROXENOS") || k.contains("BENCH"))
        );
    }

    #[test]
    fn scratch_dir_is_removed() {
        let s = Scratch::new().unwrap();
        let path = s.0.clone();
        assert!(path.is_dir());
        drop(s);
        assert!(!path.exists());
    }
}
