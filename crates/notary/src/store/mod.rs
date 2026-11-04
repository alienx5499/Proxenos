//! Persistence: one SQLite database for deals, events, sealed limits and idempotency.

pub mod sqlite;
mod sqlite_rows;
mod sqlite_save;
mod sqlite_startup;
#[cfg(test)]
mod sqlite_tests;
