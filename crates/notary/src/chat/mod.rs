//! Chat interface: grammar, rendering, and the router that maps messages to use cases.

pub mod canned;
mod cards;
mod desk_replies;
pub(crate) mod format;
pub mod grammar;
pub mod inbound;
mod kickoff;
mod moves;
mod parties;
mod rate;
pub mod render;
mod replies;
pub mod router;
mod router_ports;
mod sheet;
