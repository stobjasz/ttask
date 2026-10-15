//! Reminders, dependencies and the time log.

use std::collections::{HashSet, VecDeque};

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use ttask_core::TaskId;

use crate::db::Db;
use crate::{Result, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reminder {
    pub id: i64,
    pub task_id: TaskId,
    pub fire_at: i64,
    pub fired_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimeEntry {
    pub id: i64,
    pub task_id: TaskId,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

impl TimeEntry {
    pub fn minutes(&self, now_ms: i64) -> i64 {
        (self.ended_at.unwrap_or(now_ms) - self.started_at).max(0) / 60_000
    }
}

impl Db<'_> {
    // --- reminders ---

    pub fn add_reminder(&self, task: TaskId, fire_at: i64) -> Result<Reminder> {
        self.task(task)?;
        self.conn.execute(
            "INSERT INTO reminders (task_id, fire_at) VALUES (?1, ?2)",
            params![task, fire_at],
        )?;
        Ok(Reminder {
            id: self.conn.last_insert_rowid(),
            task_id: task,
            fire_at,
            fired_at: None,
        })
    }

    pub fn clear_reminders(&self, task: TaskId) -> Result<()> {
        self.conn
            .execute("DELETE FROM reminders WHERE task_id = ?1", [task])?;
        Ok(())
    }

    pub fn reminders_for(&self, task: TaskId) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, fire_at, fired_at FROM reminders WHERE task_id = ?1 ORDER BY fire_at",
        )?;
        let rows = stmt.query_map([task], reminder_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Unfired reminders due by `now_ms` on open, live tasks.
    pub fn due_reminders(&self, now_ms: i64) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.task_id, r.fire_at, r.fired_at FROM reminders r
               JOIN tasks t ON t.id = r.task_id
              WHERE r.fired_at IS NULL AND r.fire_at <= ?1 AND t.status = 'open'
                AND t.deleted_at IS NULL
              ORDER BY r.fire_at",
        )?;
        let rows = stmt.query_map([now_ms], reminder_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Unfired reminders scheduled after `now_ms`, soonest first.
    pub fn upcoming_reminders(&self, now_ms: i64, limit: u32) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.task_id, r.fire_at, r.fired_at FROM reminders r
               JOIN tasks t ON t.id = r.task_id
              WHERE r.fired_at IS NULL AND r.fire_at > ?1 AND t.status = 'open'
                AND t.deleted_at IS NULL
              ORDER BY r.fire_at LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now_ms, limit], reminder_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_reminder_fired(&self, id: i64, now_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET fired_at = ?2 WHERE id = ?1",
            params![id, now_ms],
        )?;
        Ok(())
    }

    // --- dependencies ---

    /// Makes `blocked` wait on `blocker`. Refuses self-dependencies and cycles.
    pub fn add_dependency(&self, blocker: TaskId, blocked: TaskId) -> Result<()> {
        if blocker == blocked {
            return Err(StoreError::Invalid("a task cannot block itself".into()));
        }
        self.task(blocker)?;
        self.task(blocked)?;
        if self.depends_on(blocker, blocked)? {
            return Err(StoreError::Invalid(format!(
                "task {blocker} already waits on task {blocked}; that would be a cycle"
            )));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO task_deps (blocker_id, blocked_id) VALUES (?1, ?2)",
            params![blocker, blocked],
        )?;
        Ok(())
    }

    pub fn remove_dependency(&self, blocker: TaskId, blocked: TaskId) -> Result<()> {
        let removed = self.conn.execute(
            "DELETE FROM task_deps WHERE blocker_id = ?1 AND blocked_id = ?2",
            params![blocker, blocked],
        )?;
        if removed == 0 {
            return Err(StoreError::NotFound(format!(
                "dependency of task {blocked} on task {blocker}"
            )));
        }
        Ok(())
    }

    /// Tasks `id` waits on.
    pub fn blockers(&self, id: TaskId) -> Result<Vec<TaskId>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.blocker_id FROM task_deps d JOIN tasks t ON t.id = d.blocker_id
              WHERE d.blocked_id = ?1 AND t.deleted_at IS NULL ORDER BY d.blocker_id",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Tasks waiting on `id`.
    pub fn dependents(&self, id: TaskId) -> Result<Vec<TaskId>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.blocked_id FROM task_deps d JOIN tasks t ON t.id = d.blocked_id
              WHERE d.blocker_id = ?1 AND t.deleted_at IS NULL ORDER BY d.blocked_id",
        )?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Whether `from` (transitively) waits on `to`.
    fn depends_on(&self, from: TaskId, to: TaskId) -> Result<bool> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([from]);
        while let Some(current) = queue.pop_front() {
            if current == to {
                return Ok(true);
            }
            if seen.insert(current) {
                queue.extend(self.blockers(current)?);
            }
        }
        Ok(false)
    }

    // --- time log ---

    /// Starts timing `task`, stopping any running entry first.
    pub fn start_timer(&self, task: TaskId, now_ms: i64) -> Result<(Option<TimeEntry>, TimeEntry)> {
        self.task(task)?;
        let stopped = self.stop_timer(now_ms)?;
        self.conn.execute(
            "INSERT INTO time_entries (task_id, started_at) VALUES (?1, ?2)",
            params![task, now_ms],
        )?;
        Ok((
            stopped,
            TimeEntry {
                id: self.conn.last_insert_rowid(),
                task_id: task,
                started_at: now_ms,
                ended_at: None,
            },
        ))
    }

    pub fn stop_timer(&self, now_ms: i64) -> Result<Option<TimeEntry>> {
        let Some(mut running) = self.running_timer()? else {
            return Ok(None);
        };
        self.conn.execute(
            "UPDATE time_entries SET ended_at = ?2 WHERE id = ?1",
            params![running.id, now_ms],
        )?;
        running.ended_at = Some(now_ms);
        Ok(Some(running))
    }

    pub fn running_timer(&self) -> Result<Option<TimeEntry>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, task_id, started_at, ended_at FROM time_entries
                  WHERE ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
                [],
                time_entry_from_row,
            )
            .optional()?)
    }

    pub fn time_entries(&self, task: TaskId) -> Result<Vec<TimeEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, started_at, ended_at FROM time_entries WHERE task_id = ?1
              ORDER BY started_at",
        )?;
        let rows = stmt.query_map([task], time_entry_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Entries that overlap `[from_ms, to_ms)`, for reports.
    pub fn time_entries_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<TimeEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, started_at, ended_at FROM time_entries
              WHERE started_at < ?2 AND coalesce(ended_at, ?2) > ?1 ORDER BY started_at",
        )?;
        let rows = stmt.query_map(params![from_ms, to_ms], time_entry_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn reminder_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: r.get(0)?,
        task_id: r.get(1)?,
        fire_at: r.get(2)?,
        fired_at: r.get(3)?,
    })
}

fn time_entry_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TimeEntry> {
    Ok(TimeEntry {
        id: r.get(0)?,
        task_id: r.get(1)?,
        started_at: r.get(2)?,
        ended_at: r.get(3)?,
    })
}
