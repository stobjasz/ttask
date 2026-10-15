use std::time::{SystemTime, UNIX_EPOCH};

/// Source of "now", injected so date-dependent logic is testable.
pub trait Clock: Send + Sync {
    /// Current instant as UTC Unix milliseconds.
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before 1970");
        i64::try_from(since_epoch.as_millis()).expect("timestamp overflows i64")
    }
}

/// A clock frozen at one instant, for tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}
