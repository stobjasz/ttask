//! Raw rows for reports; the services layer buckets them by week.

use crate::Result;
use crate::db::Db;

/// A task with a due date, as far as the overdue trend needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueRow {
    /// `YYYY-MM-DD`.
    pub due_date: String,
    pub created_at: i64,
    /// When it was done or cancelled.
    pub closed_at: Option<i64>,
}

/// One time-log entry with its task's project (`None`: Inbox).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRow {
    pub project: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// A done task with an estimate and the time tracked on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstimateRow {
    pub completed_at: i64,
    pub estimate_min: u32,
    /// Finished entries only.
    pub tracked_ms: i64,
}

impl Db<'_> {
    /// Done (not cancelled) tasks completed since `since_ms` that have both
    /// an estimate and some tracked time.
    pub fn estimate_rows_since(&self, since_ms: i64) -> Result<Vec<EstimateRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.completed_at, t.estimate_min, sum(e.ended_at - e.started_at)
               FROM tasks t JOIN time_entries e ON e.task_id = t.id AND e.ended_at IS NOT NULL
              WHERE t.status = 'done' AND t.deleted_at IS NULL AND t.completed_at >= ?1
                AND t.estimate_min > 0
              GROUP BY t.id",
        )?;
        let rows = stmt.query_map([since_ms], |r| {
            Ok(EstimateRow {
                completed_at: r.get(0)?,
                estimate_min: r.get(1)?,
                tracked_ms: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Completion times of done (not cancelled) tasks since `since_ms`.
    pub fn completions_since(&self, since_ms: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT completed_at FROM tasks
              WHERE status = 'done' AND deleted_at IS NULL AND completed_at >= ?1",
        )?;
        let rows = stmt.query_map([since_ms], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every live task with a due date.
    pub fn due_rows(&self) -> Result<Vec<DueRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT due_date, created_at, completed_at FROM tasks
              WHERE due_date IS NOT NULL AND deleted_at IS NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DueRow {
                due_date: r.get(0)?,
                created_at: r.get(1)?,
                closed_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Time entries overlapping `[since_ms, ∞)` on live tasks.
    pub fn time_rows_since(&self, since_ms: i64) -> Result<Vec<TimeRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT CASE WHEN p.id IS NULL THEN NULL
                         WHEN pp.id IS NULL THEN p.name
                         ELSE pp.name || '/' || p.name END,
                    e.started_at, e.ended_at
               FROM time_entries e
               JOIN tasks t ON t.id = e.task_id AND t.deleted_at IS NULL
               LEFT JOIN projects p ON p.id = t.project_id
               LEFT JOIN projects pp ON pp.id = p.parent_id
              WHERE e.ended_at IS NULL OR e.ended_at >= ?1",
        )?;
        let rows = stmt.query_map([since_ms], |r| {
            Ok(TimeRow {
                project: r.get(0)?,
                started_at: r.get(1)?,
                ended_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
