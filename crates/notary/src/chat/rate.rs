//! Per-sender rate limit (threat T9, flooding): a fixed window of commands per minute.

use std::collections::HashMap;
use std::sync::Mutex;

use proxenos_domain::party::ParticipantId;
use proxenos_domain::timestamp::Timestamp;

const WINDOW_SECS: i64 = 60;
/// Commands one sender may send per window; a negotiator's turn needs a handful.
const PER_WINDOW: u32 = 20;
/// Senders tracked before expired windows are swept.
const SWEEP_AT: usize = 1_024;

#[derive(Default)]
pub struct RateLimit(Mutex<HashMap<ParticipantId, (i64, u32)>>);

impl RateLimit {
    /// Counts one command from `sender`; false once the sender is over the limit.
    pub fn allow(&self, sender: &ParticipantId, now: Timestamp) -> bool {
        let t = now.seconds();
        let mut map = self.0.lock().expect("rate lock");
        if map.len() >= SWEEP_AT {
            map.retain(|_, (start, _)| t - *start < WINDOW_SECS);
        }
        let (start, count) = map.entry(sender.clone()).or_insert((t, 0));
        if t - *start >= WINDOW_SECS {
            (*start, *count) = (t, 0);
        }
        *count += 1;
        *count <= PER_WINDOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_each_sender_per_window() {
        let rl = RateLimit::default();
        let (a, b) = (
            ParticipantId::new("a").unwrap(),
            ParticipantId::new("b").unwrap(),
        );
        assert!((0..PER_WINDOW).all(|_| rl.allow(&a, Timestamp(0))));
        assert!(!rl.allow(&a, Timestamp(WINDOW_SECS - 1)));
        assert!(rl.allow(&b, Timestamp(1)), "other senders are unaffected");
        assert!(rl.allow(&a, Timestamp(WINDOW_SECS)), "a new window resets");
    }
}
