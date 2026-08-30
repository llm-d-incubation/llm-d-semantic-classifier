//! A minimal consecutive-failure circuit breaker for the best-effort L2 cache.
//! After `failure_threshold` consecutive failures it opens for `cooldown`,
//! so a dead Redis costs one probe per cooldown window, not one per request.

use std::sync::Mutex;
use std::time::{Duration, Instant};

struct State {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

pub struct CircuitBreaker {
    failure_threshold: u32,
    cooldown: Duration,
    state: Mutex<State>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        CircuitBreaker {
            failure_threshold: failure_threshold.max(1),
            cooldown,
            state: Mutex::new(State { consecutive_failures: 0, open_until: None }),
        }
    }

    /// True if a call may proceed (closed, or half-open after cooldown).
    pub fn allow(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        match s.open_until {
            Some(t) if Instant::now() < t => false,
            Some(_) => {
                // Cooldown elapsed: half-open, let one probe through.
                s.open_until = None;
                s.consecutive_failures = 0;
                true
            }
            None => true,
        }
    }

    pub fn record_success(&self) {
        let mut s = self.state.lock().unwrap();
        s.consecutive_failures = 0;
        s.open_until = None;
    }

    pub fn record_failure(&self) {
        let mut s = self.state.lock().unwrap();
        s.consecutive_failures += 1;
        if s.consecutive_failures >= self.failure_threshold {
            s.open_until = Some(Instant::now() + self.cooldown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn opens_after_threshold_and_closes_after_cooldown() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(50));
        assert!(cb.allow(), "starts closed");
        cb.record_failure();
        assert!(cb.allow(), "one failure below threshold");
        cb.record_failure();
        assert!(!cb.allow(), "opens at threshold");
        std::thread::sleep(Duration::from_millis(60));
        assert!(cb.allow(), "closes (half-open) after cooldown");
    }

    #[test]
    fn success_resets_failures() {
        let cb = CircuitBreaker::new(2, Duration::from_secs(10));
        cb.record_failure();
        cb.record_success();
        cb.record_failure();
        assert!(cb.allow(), "success cleared the earlier failure");
    }
}
