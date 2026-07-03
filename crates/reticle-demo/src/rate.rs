//! A per-key sliding-window rate limiter.
//!
//! Each key (a source IP) is allowed at most `limit` accepted events in any
//! trailing `window`. The limiter keeps, per key, the timestamps of the events
//! still inside the window; on each check it drops expired timestamps, and
//! accepts only if fewer than `limit` remain. A true sliding window (rather than
//! a fixed calendar-minute bucket) prevents the burst-at-the-boundary hole where
//! `2 * limit` events land across two adjacent fixed windows.
//!
//! The limiter is deliberately time-injectable: [`RateLimiter::check_at`] takes
//! the "now" instant so tests can drive the window without sleeping. The public
//! [`RateLimiter::check`] uses the real clock.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A sliding-window counter keyed by an arbitrary string.
#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    events: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Builds a limiter allowing `limit` events per `window` per key.
    #[must_use]
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            events: Mutex::new(HashMap::new()),
        }
    }

    /// Records an event for `key` at real-clock `now` and reports whether it is
    /// within the limit. See [`RateLimiter::check_at`].
    pub fn check(&self, key: &str) -> bool {
        self.check_at(key, Instant::now())
    }

    /// Records an event for `key` at instant `now` and reports whether it is
    /// allowed.
    ///
    /// Returns `true` if, after evicting events older than `window`, this key has
    /// made fewer than `limit` events; in that case the event is recorded.
    /// Returns `false` (and records nothing) when the key is already at the
    /// limit, so a rejected request does not itself extend the window.
    ///
    /// A `limit` of zero rejects everything.
    pub fn check_at(&self, key: &str, now: Instant) -> bool {
        let mut events = self.events.lock().expect("rate limiter mutex poisoned");
        let bucket = events.entry(key.to_owned()).or_default();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        while bucket.front().is_some_and(|&t| t <= cutoff) {
            bucket.pop_front();
        }
        if bucket.len() as u32 >= self.limit {
            return false;
        }
        bucket.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;
    use std::time::{Duration, Instant};

    #[test]
    fn allows_up_to_limit_then_rejects() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(rl.check_at("ip", t0));
        assert!(rl.check_at("ip", t0));
        assert!(rl.check_at("ip", t0));
        assert!(
            !rl.check_at("ip", t0),
            "fourth event in the window is rejected"
        );
    }

    #[test]
    fn window_slides_and_frees_capacity() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(rl.check_at("ip", t0));
        assert!(rl.check_at("ip", t0 + Duration::from_secs(10)));
        assert!(!rl.check_at("ip", t0 + Duration::from_secs(20)));
        // Past the first event's expiry, one slot frees up.
        assert!(rl.check_at("ip", t0 + Duration::from_secs(61)));
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::new(1, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(rl.check_at("a", t0));
        assert!(rl.check_at("b", t0));
        assert!(!rl.check_at("a", t0));
    }

    #[test]
    fn zero_limit_rejects_all() {
        let rl = RateLimiter::new(0, Duration::from_secs(60));
        assert!(!rl.check_at("ip", Instant::now()));
    }
}
