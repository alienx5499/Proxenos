//! Proxenos notary: parse the command line and run it.

mod adapters;
mod chat;
mod cli;
mod config;
mod crypto;
mod http;
mod serve;
mod store;
mod switch_admin;
mod vault;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::config::Config;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve => match Config::from_env() {
            Ok(cfg) => serve::serve(cfg).await,
            Err(e) => Err(e.to_string()),
        },
        Command::Admit(args) => cli::admit::run(&args)
            .await
            .map(|group| println!("admitted: room group {group}"))
            .map_err(|e| e.0),
        Command::Register(args) => cli::register::run(&args)
            .await
            .map(|id| println!("{id}"))
            .map_err(|e| e.0),
        Command::Verify(args) => cli::verify::run(&args)
            .map(|line| println!("{line}"))
            .map_err(|e| e.0),
        Command::Fixture(args) => match Config::from_env() {
            Ok(cfg) => cli::fixture::run(&args, &cfg)
                .await
                .map(|line| println!("{line}"))
                .map_err(|e| e.0),
            Err(e) => Err(e.to_string()),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("proxenos: {e}");
            ExitCode::FAILURE
        }
    }
}
