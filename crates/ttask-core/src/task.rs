use std::fmt;
use std::str::FromStr;

use serde::Serialize;

pub type TaskId = i64;

/// Generates a new ULID, the stable identifier used across export/import.
pub fn new_uid() -> String {
    ulid::Ulid::generate().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Done,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseStatusError(pub String);

impl fmt::Display for ParseStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown task status `{}`", self.0)
    }
}

impl std::error::Error for ParseStatusError {}

impl FromStr for Status {
    type Err = ParseStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Status::Open),
            "done" => Ok(Status::Done),
            "cancelled" => Ok(Status::Cancelled),
            other => Err(ParseStatusError(other.to_owned())),
        }
    }
}

/// Priority 0 (none) to 3 (urgent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize)]
#[serde(transparent)]
pub struct Priority(u8);

impl Priority {
    pub const NONE: Priority = Priority(0);
    pub const MAX: u8 = 3;

    /// Returns `None` when `value` is above [`Priority::MAX`].
    pub fn new(value: u8) -> Option<Priority> {
        (value <= Self::MAX).then_some(Priority(value))
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

/// Fields a caller supplies when creating a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub priority: Priority,
}

impl NewTask {
    pub fn titled(title: impl Into<String>) -> NewTask {
        NewTask {
            title: title.into(),
            notes: String::new(),
            priority: Priority::NONE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: TaskId,
    pub uid: String,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub priority: Priority,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_string_form() {
        for status in [Status::Open, Status::Done, Status::Cancelled] {
            assert_eq!(status.as_str().parse::<Status>(), Ok(status));
        }
        assert!("finished".parse::<Status>().is_err());
    }

    #[test]
    fn priority_rejects_values_above_max() {
        assert_eq!(Priority::new(3).map(Priority::value), Some(3));
        assert_eq!(Priority::new(4), None);
    }

    #[test]
    fn uids_are_unique_ulids() {
        let a = new_uid();
        let b = new_uid();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }
}
