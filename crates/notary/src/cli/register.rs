//! `proxenos register`: one-time registration of the notary as an always-on Switch agent.
//!
//! The registration key must be minted by the operator (`POST /gateway/api-keys`), so the
//! operator owns the notary and its owner-only rule admits the operator's kickoffs.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use proxenos_switch_client::register::{Credentials, Registration, register};

use super::{CliError, read_secret};

#[derive(Debug, clap::Args)]
pub struct RegisterArgs {
    #[arg(long, env = "SWITCH_URL")]
    pub switch_url: String,
    /// File holding the operator's registration key (`-` for stdin); else
    /// `SWITCH_REGISTRATION_KEY`.
    #[arg(long)]
    pub registration_key_file: Option<PathBuf>,
    /// Agent name in Switch.
    #[arg(long, default_value = "notary")]
    pub name: String,
    /// Where the `SWITCH_AGENT_ID` and `SWITCH_AGENT_KEY` lines go (created with mode 0600).
    #[arg(long)]
    pub out: PathBuf,
    /// Overwrite `--out` if it exists.
    #[arg(long)]
    pub force: bool,
}

/// Returns the new agent id; the key is only ever written to `--out`.
pub async fn run(args: &RegisterArgs) -> Result<String, CliError> {
    let key = read_secret(
        "SWITCH_REGISTRATION_KEY",
        args.registration_key_file.as_deref(),
    )?
    .ok_or_else(|| {
        CliError("set SWITCH_REGISTRATION_KEY or pass --registration-key-file".into())
    })?;
    if args.out.exists() && !args.force {
        return Err(CliError(format!(
            "{} exists; pass --force to overwrite it",
            args.out.display()
        )));
    }
    let reg = Registration {
        name: args.name.clone(),
        description: "Proxenos notary: sealed limits, offers and signatures".into(),
        display_name: None,
        connector_type: "proxenos".into(),
        overwrite: false,
    };
    let creds = register(&args.switch_url, key.expose(), &reg)
        .await
        .map_err(|e| CliError(format!("registration failed: {e}")))?;
    write_out(args, &creds)?;
    Ok(creds.agent_id)
}

fn write_out(args: &RegisterArgs, creds: &Credentials) -> Result<(), CliError> {
    let err = |e: std::io::Error| CliError(format!("{}: {e}", args.out.display()));
    let mut opts = OpenOptions::new();
    opts.write(true).mode(0o600);
    if args.force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    let mut f = opts.open(&args.out).map_err(err)?;
    // A pre-existing file keeps its old mode on open; tighten it before writing the key.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(err)?;
    writeln!(
        f,
        "SWITCH_AGENT_ID={}\nSWITCH_AGENT_KEY={}",
        creds.agent_id, creds.api_key
    )
    .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(base: &str, out: PathBuf, force: bool) -> (RegisterArgs, PathBuf) {
        let key = out.with_extension("key");
        std::fs::write(&key, "regkey\n").unwrap();
        let a = RegisterArgs {
            switch_url: base.into(),
            registration_key_file: Some(key.clone()),
            name: "notary".into(),
            out,
            force,
        };
        (a, key)
    }

    #[tokio::test]
    async fn registers_always_on_and_writes_a_private_env_file() {
        let mock = crate::adapters::mock::Mock::start(|_, _| {
            (200, r#"{"id":"agent-1","api_key":"sk-secret"}"#.to_owned())
        })
        .await;
        let dir = std::env::temp_dir().join(format!("proxenos-reg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("notary.env");
        let (a, _) = args(&mock.base, out.clone(), false);
        assert_eq!(run(&a).await.unwrap(), "agent-1");
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            text,
            "SWITCH_AGENT_ID=agent-1\nSWITCH_AGENT_KEY=sk-secret\n"
        );
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let seen = &mock.seen_path("/agents")[0];
        assert_eq!(seen.header("authorization").unwrap(), "Bearer regkey");
        let body = seen.json();
        assert_eq!(body["name"], "notary");
        assert_eq!(body["integration_profile"]["connection_model"], "always_on");

        // An existing file is refused before any request, and replaced only with --force.
        let err = run(&a).await.unwrap_err();
        assert!(err.0.contains("--force"), "{err}");
        assert_eq!(mock.seen_path("/agents").len(), 1);
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (a, _) = args(&mock.base, out.clone(), true);
        run(&a).await.unwrap();
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
