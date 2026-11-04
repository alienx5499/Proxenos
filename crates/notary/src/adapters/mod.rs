//! Production adapters for the app ports that talk to Switch, plus the system clock.

#[cfg(test)]
pub mod mock;
mod rfc3339;
pub mod switch;
#[cfg(test)]
mod switch_tests;

use std::time::{SystemTime, UNIX_EPOCH};

use proxenos_app::ports::Clock;
use proxenos_domain::timestamp::Timestamp;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Timestamp(i64::try_from(secs).unwrap_or(i64::MAX))
    }
}
