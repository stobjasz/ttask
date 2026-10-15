use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

/// Source of "now" and the local time zone, injected so date-dependent logic
/// (overdue, today, recurrence) is testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;

    fn time_zone(&self) -> TimeZone;

    /// Current instant as UTC Unix milliseconds.
    fn now_ms(&self) -> i64 {
        self.now().as_millisecond()
    }

    fn now_zoned(&self) -> Zoned {
        self.now().to_zoned(self.time_zone())
    }

    /// The local calendar date.
    fn today(&self) -> Date {
        self.now_zoned().date()
    }
}

/// The real clock in the system time zone (looked up once).
#[derive(Debug, Clone)]
pub struct SystemClock {
    tz: TimeZone,
}

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock {
            tz: TimeZone::system(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn time_zone(&self) -> TimeZone {
        self.tz.clone()
    }
}

/// A clock frozen at one instant, for tests.
#[derive(Debug, Clone)]
pub struct FixedClock {
    pub at: Timestamp,
    pub tz: TimeZone,
}

impl FixedClock {
    /// Frozen at `ms` Unix milliseconds, in UTC.
    pub fn at_ms(ms: i64) -> FixedClock {
        FixedClock {
            at: Timestamp::from_millisecond(ms).expect("valid timestamp"),
            tz: TimeZone::UTC,
        }
    }

    /// Frozen at noon UTC on the given date.
    pub fn on(date: Date) -> FixedClock {
        let at = date
            .at(12, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .expect("valid date")
            .timestamp();
        FixedClock {
            at,
            tz: TimeZone::UTC,
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.at
    }

    fn time_zone(&self) -> TimeZone {
        self.tz.clone()
    }
}

/// UTC milliseconds at local midnight starting `date`.
pub fn start_of_day_ms(date: Date, tz: &TimeZone) -> i64 {
    date.to_zoned(tz.clone())
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or_else(|_| {
            date.at(0, 0, 0, 0)
                .to_zoned(TimeZone::UTC)
                .expect("date in range")
                .timestamp()
                .as_millisecond()
        })
}

/// The local date an instant falls on.
pub fn date_of_ms(ms: i64, tz: &TimeZone) -> Date {
    Timestamp::from_millisecond(ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(tz.clone())
        .date()
}
