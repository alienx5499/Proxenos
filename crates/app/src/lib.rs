//! Proxenos application layer: use cases over ports.

pub mod commands;
pub mod error;
#[cfg(any(test, feature = "test-fakes"))]
pub mod fakes;
pub mod ports;
pub mod queries;
pub mod receipt;
pub mod secret;
pub mod stored;

pub use error::AppError;

#[cfg(test)]
mod tests;
