use std::fmt;
use std::str::FromStr;

use jiff::civil::{Date, Time};
use serde::{Deserialize, Serialize};

use crate::recur::RecurMode;

pub type TaskId = i64;
pub type ProjectId = i64;

/// Generates a new ULID, the stable identifier used across export/import.
pub fn new_uid() -> String {
    ulid::Ulid::generate().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
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
            "cancelled" | "canceled" => Ok(Status::Cancelled),
            other => Err(ParseStatusError(other.to_owned())),
        }
    }
}

/// Priority 0 (low, the default) to 3 (urgent).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, Hash,
)]
#[serde(transparent)]
pub struct Priority(u8);

impl Priority {
    pub const NONE: Priority = Priority(0);
    pub const MAX: u8 = 3;
    const NAMES: [&'static str; 4] = ["low", "medium", "high", "urgent"];

    /// Returns `None` when `value` is above [`Priority::MAX`].
    pub fn new(value: u8) -> Option<Priority> {
        (value <= Self::MAX).then_some(Priority(value))
    }

    pub fn value(self) -> u8 {
        self.0
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self.0 as usize]
    }

    /// `low`, `med`/`medium`, `high`, `urgent` (lowercase).
    pub fn from_name(name: &str) -> Option<Priority> {
        Some(Priority(match name {
            "low" | "none" => 0,
            "med" | "medium" => 1,
            "high" => 2,
            "urgent" => 3,
            _ => return None,
        }))
    }

    /// `""`, `"!"`, `"!!"`, `"!!!"`.
    pub fn marks(self) -> &'static str {
        ["", "!", "!!", "!!!"][self.0 as usize]
    }
}

/// Fields a caller supplies when creating a task.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub priority: Priority,
    pub project_id: Option<ProjectId>,
    pub parent_id: Option<TaskId>,
    pub due_date: Option<Date>,
    pub due_time: Option<Time>,
    pub scheduled_date: Option<Date>,
    pub estimate_min: Option<u32>,
    pub recur_rule: Option<String>,
    pub recur_mode: Option<RecurMode>,
    pub recur_series: Option<String>,
    pub wait_until: Option<Date>,
    pub someday: bool,
    pub tags: Vec<String>,
}

impl NewTask {
    pub fn titled(title: impl Into<String>) -> NewTask {
        NewTask {
            title: title.into(),
            ..NewTask::default()
        }
    }
}

/// The same task again, open, in the same recurrence series.
impl From<&Task> for NewTask {
    fn from(t: &Task) -> NewTask {
        NewTask {
            title: t.title.clone(),
            notes: t.notes.clone(),
            priority: t.priority,
            project_id: t.project_id,
            parent_id: t.parent_id,
            due_date: t.due_date,
            due_time: t.due_time,
            scheduled_date: t.scheduled_date,
            estimate_min: t.estimate_min,
            recur_rule: t.recur_rule.clone(),
            recur_mode: t.recur_mode,
            recur_series: t.recur_series.clone(),
            wait_until: t.wait_until,
            someday: t.someday,
            tags: t.tags.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: TaskId,
    pub uid: String,
    pub parent_id: Option<TaskId>,
    pub project_id: Option<ProjectId>,
    /// `name` or `parent/name`.
    pub project: Option<String>,
    pub title: String,
    pub notes: String,
    pub status: Status,
    pub priority: Priority,
    pub due_date: Option<Date>,
    pub due_time: Option<Time>,
    pub scheduled_date: Option<Date>,
    pub estimate_min: Option<u32>,
    pub recur_rule: Option<String>,
    pub recur_mode: Option<RecurMode>,
    pub recur_series: Option<String>,
    /// Hidden from the usual views until this date.
    pub wait_until: Option<Date>,
    /// Parked indefinitely; only the Someday view shows it.
    pub someday: bool,
    pub tags: Vec<String>,
    pub sort_order: f64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub subtasks: Progress,
    pub checklist: Progress,
    /// Waiting on an open task (dependencies).
    pub blocked: bool,
    /// An open task waits on this one.
    pub blocking: bool,
    /// Filled in by the services layer (see `urgency`); 0 when not computed.
    pub urgency: f64,
}

impl Task {
    pub fn is_open(&self) -> bool {
        self.status == Status::Open
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.is_open() && self.due_date.is_some_and(|d| d < today)
    }

    /// Hidden by a future `wait:` date.
    pub fn is_waiting(&self, today: Date) -> bool {
        self.wait_until.is_some_and(|d| d > today)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Project {
    pub id: ProjectId,
    pub uid: String,
    pub parent_id: Option<ProjectId>,
    pub name: String,
    /// `name` or `parent/name`.
    pub path: String,
    pub color: Option<String>,
    pub archived: bool,
    pub open_tasks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagInfo {
    pub name: String,
    pub color: Option<String>,
    pub open_tasks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: i64,
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedFilter {
    pub id: i64,
    pub name: String,
    pub query: String,
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
    fn priority_names_and_bounds() {
        assert_eq!(Priority::new(3).map(Priority::value), Some(3));
        assert_eq!(Priority::new(4), None);
        assert_eq!(Priority::from_name("high"), Priority::new(2));
        assert_eq!(Priority::new(2).unwrap().name(), "high");
        assert_eq!(Priority::new(3).unwrap().marks(), "!!!");
    }

    #[test]
    fn uids_are_unique_ulids() {
        let a = new_uid();
        let b = new_uid();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }
}
