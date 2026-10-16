//! Task snapshots and the event journal behind undo/redo and history.
//!
//! Every user action is one batch: for each task it touches, the journal keeps
//! a full snapshot before and after. Undo restores the "before" snapshots of
//! the newest batch; redo re-applies the "after" snapshots. A new action
//! discards undone batches, as in any editor.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ttask_core::TaskId;

use crate::Result;
use crate::db::Db;

/// `(kind, before, after)` of one journal entry.
type EventRow = (String, Option<String>, Option<String>);

/// Everything needed to put a task back exactly as it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub uid: String,
    pub project_id: Option<i64>,
    pub parent_id: Option<TaskId>,
    pub title: String,
    pub notes: String,
    pub status: String,
    pub priority: u8,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub scheduled_date: Option<String>,
    pub estimate_min: Option<u32>,
    pub recur_rule: Option<String>,
    pub recur_mode: Option<String>,
    pub recur_series: Option<String>,
    pub sort_order: f64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub wait_until: Option<String>,
    #[serde(default)]
    pub someday: bool,
    pub tags: Vec<String>,
    pub checklist: Vec<SnapshotChecklistItem>,
    pub reminders: Vec<SnapshotReminder>,
    pub blockers: Vec<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotChecklistItem {
    pub text: String,
    pub checked: bool,
    pub sort_order: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotReminder {
    pub fire_at: i64,
    pub fired_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Created,
    Updated,
    Completed,
    Reopened,
    Deleted,
    Spawned,
    Imported,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Created => "created",
            EventKind::Updated => "updated",
            EventKind::Completed => "completed",
            EventKind::Reopened => "reopened",
            EventKind::Deleted => "deleted",
            EventKind::Spawned => "spawned",
            EventKind::Imported => "imported",
        }
    }
}

/// One line of a task's history.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub at: i64,
    pub kind: String,
    /// Field names whose value changed.
    pub changed: Vec<String>,
}

/// What an undo or redo touched.
#[derive(Debug, Clone, Serialize)]
pub struct UndoOutcome {
    pub batch: i64,
    pub kinds: Vec<String>,
    pub titles: Vec<String>,
}

impl Db<'_> {
    /// Snapshot of a task row, including soft-deleted ones.
    pub fn snapshot(&self, id: TaskId) -> Result<Option<TaskSnapshot>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, uid, project_id, parent_id, title, notes, status, priority, due_date,
                        due_time, scheduled_date, estimate_min, recur_rule, recur_mode, recur_series,
                        sort_order, created_at, updated_at, completed_at, deleted_at,
                        wait_until, someday
                   FROM tasks WHERE id = ?1",
                [id],
                |r| {
                    Ok(TaskSnapshot {
                        id: r.get(0)?,
                        uid: r.get(1)?,
                        project_id: r.get(2)?,
                        parent_id: r.get(3)?,
                        title: r.get(4)?,
                        notes: r.get(5)?,
                        status: r.get(6)?,
                        priority: r.get(7)?,
                        due_date: r.get(8)?,
                        due_time: r.get(9)?,
                        scheduled_date: r.get(10)?,
                        estimate_min: r.get(11)?,
                        recur_rule: r.get(12)?,
                        recur_mode: r.get(13)?,
                        recur_series: r.get(14)?,
                        sort_order: r.get(15)?,
                        created_at: r.get(16)?,
                        updated_at: r.get(17)?,
                        completed_at: r.get(18)?,
                        deleted_at: r.get(19)?,
                        wait_until: r.get(20)?,
                        someday: r.get(21)?,
                        tags: Vec::new(),
                        checklist: Vec::new(),
                        reminders: Vec::new(),
                        blockers: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut snap) = row else {
            return Ok(None);
        };
        snap.tags = self
            .conn
            .prepare(
                "SELECT g.name FROM task_tags tt JOIN tags g ON g.id = tt.tag_id
                  WHERE tt.task_id = ?1 ORDER BY g.name COLLATE NOCASE",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        snap.checklist = self
            .conn
            .prepare(
                "SELECT text, checked, sort_order FROM checklist_items WHERE task_id = ?1
                  ORDER BY sort_order, id",
            )?
            .query_map([id], |r| {
                Ok(SnapshotChecklistItem {
                    text: r.get(0)?,
                    checked: r.get(1)?,
                    sort_order: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        snap.reminders = self
            .conn
            .prepare("SELECT fire_at, fired_at FROM reminders WHERE task_id = ?1 ORDER BY fire_at")?
            .query_map([id], |r| {
                Ok(SnapshotReminder {
                    fire_at: r.get(0)?,
                    fired_at: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        snap.blockers = self
            .conn
            .prepare("SELECT blocker_id FROM task_deps WHERE blocked_id = ?1 ORDER BY blocker_id")?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(snap))
    }

    /// Writes a snapshot back: updates the row (or recreates it with the same
    /// id) and replaces its tags, checklist, reminders and blockers.
    pub fn restore(&self, s: &TaskSnapshot) -> Result<()> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
            [s.id],
            |r| r.get(0),
        )?;
        let values = params![
            s.id,
            s.uid,
            s.project_id,
            s.parent_id,
            s.title,
            s.notes,
            s.status,
            s.priority,
            s.due_date,
            s.due_time,
            s.scheduled_date,
            s.estimate_min,
            s.recur_rule,
            s.recur_mode,
            s.recur_series,
            s.sort_order,
            s.created_at,
            s.updated_at,
            s.completed_at,
            s.deleted_at,
            s.wait_until,
            s.someday
        ];
        if exists {
            self.conn.execute(
                "UPDATE tasks SET uid = ?2, project_id = ?3, parent_id = ?4, title = ?5, notes = ?6,
                    status = ?7, priority = ?8, due_date = ?9, due_time = ?10, scheduled_date = ?11,
                    estimate_min = ?12, recur_rule = ?13, recur_mode = ?14, recur_series = ?15,
                    sort_order = ?16, created_at = ?17, updated_at = ?18, completed_at = ?19,
                    deleted_at = ?20, wait_until = ?21, someday = ?22
                 WHERE id = ?1",
                values,
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO tasks (id, uid, project_id, parent_id, title, notes, status, priority,
                    due_date, due_time, scheduled_date, estimate_min, recur_rule, recur_mode,
                    recur_series, sort_order, created_at, updated_at, completed_at, deleted_at,
                    wait_until, someday)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22)",
                values,
            )?;
        }
        self.set_tags(s.id, &s.tags)?;
        self.conn
            .execute("DELETE FROM checklist_items WHERE task_id = ?1", [s.id])?;
        for item in &s.checklist {
            self.conn.execute(
                "INSERT INTO checklist_items (task_id, text, checked, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![s.id, item.text, item.checked, item.sort_order],
            )?;
        }
        self.conn
            .execute("DELETE FROM reminders WHERE task_id = ?1", [s.id])?;
        for r in &s.reminders {
            self.conn.execute(
                "INSERT INTO reminders (task_id, fire_at, fired_at) VALUES (?1, ?2, ?3)",
                params![s.id, r.fire_at, r.fired_at],
            )?;
        }
        self.conn
            .execute("DELETE FROM task_deps WHERE blocked_id = ?1", [s.id])?;
        for blocker in &s.blockers {
            self.conn.execute(
                "INSERT OR IGNORE INTO task_deps (blocker_id, blocked_id)
                 SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
                params![blocker, s.id],
            )?;
        }
        Ok(())
    }

    /// Removes a task row for good (undoing its creation).
    pub fn hard_delete_task(&self, id: TaskId) -> Result<()> {
        self.conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Starts a new undoable batch, discarding anything that was undone.
    /// `(task, kind)` for each event of `batch`, in order (`kind` as
    /// [`EventKind::as_str`]).
    pub fn batch_changes(&self, batch: i64) -> Result<Vec<(TaskId, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task_id, kind FROM events WHERE batch_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([batch], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn begin_batch(&self) -> Result<i64> {
        self.conn
            .execute("DELETE FROM events WHERE undone = 1", [])?;
        Ok(self.conn.query_row(
            "SELECT coalesce(max(batch_id), 0) + 1 FROM events",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn record_event(
        &self,
        batch: i64,
        task: TaskId,
        kind: EventKind,
        before: Option<&TaskSnapshot>,
        after: Option<&TaskSnapshot>,
        at_ms: i64,
    ) -> Result<()> {
        let before = before.map(serde_json::to_string).transpose()?;
        let after = after.map(serde_json::to_string).transpose()?;
        self.conn.execute(
            "INSERT INTO events (task_id, at, kind, before, after, batch_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task, at_ms, kind.as_str(), before, after, batch],
        )?;
        Ok(())
    }

    /// Reverts the newest batch. `None` when there is nothing to undo.
    pub fn undo(&self) -> Result<Option<UndoOutcome>> {
        let batch: Option<i64> = self.conn.query_row(
            "SELECT max(batch_id) FROM events WHERE undone = 0 AND batch_id > 0",
            [],
            |r| r.get(0),
        )?;
        let Some(batch) = batch else {
            return Ok(None);
        };
        let events = self.batch_events(batch, "DESC")?;
        for (_, before, after) in &events {
            self.apply(before.as_deref(), after.as_deref())?;
        }
        self.conn
            .execute("UPDATE events SET undone = 1 WHERE batch_id = ?1", [batch])?;
        // Report in the order things happened.
        let chronological: Vec<_> = events.into_iter().rev().collect();
        Ok(Some(outcome(batch, &chronological)))
    }

    /// Re-applies the most recently undone batch.
    pub fn redo(&self) -> Result<Option<UndoOutcome>> {
        let batch: Option<i64> = self.conn.query_row(
            "SELECT min(batch_id) FROM events WHERE undone = 1",
            [],
            |r| r.get(0),
        )?;
        let Some(batch) = batch else {
            return Ok(None);
        };
        let events = self.batch_events(batch, "ASC")?;
        for (_, before, after) in &events {
            self.apply(after.as_deref(), before.as_deref())?;
        }
        self.conn
            .execute("UPDATE events SET undone = 0 WHERE batch_id = ?1", [batch])?;
        Ok(Some(outcome(batch, &events)))
    }

    /// `(kind, before, after)` for a batch, in the given id order.
    fn batch_events(&self, batch: i64, order: &str) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT kind, before, after FROM events WHERE batch_id = ?1 ORDER BY id {order}"
        ))?;
        let rows = stmt.query_map([batch], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Puts the task into state `target`; with no target the task did not
    /// exist then, so the row named by `other` is removed.
    fn apply(&self, target: Option<&str>, other: Option<&str>) -> Result<()> {
        match (target, other) {
            (Some(json), _) => self.restore(&serde_json::from_str(json)?),
            (None, Some(json)) => {
                let snap: TaskSnapshot = serde_json::from_str(json)?;
                self.hard_delete_task(snap.id)
            }
            (None, None) => Ok(()),
        }
    }

    /// The task's events, oldest first, with the fields each one changed.
    pub fn history(&self, task: TaskId) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, kind, before, after FROM events WHERE task_id = ?1 AND undone = 0
             ORDER BY at, id",
        )?;
        let rows = stmt.query_map([task], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (at, kind, before, after) = row?;
            let changed = match (before, after) {
                (Some(b), Some(a)) => changed_fields(&b, &a)?,
                _ => Vec::new(),
            };
            entries.push(HistoryEntry { at, kind, changed });
        }
        Ok(entries)
    }

    /// Whether anything can be undone / redone.
    pub fn undo_state(&self) -> Result<(bool, bool)> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM events WHERE undone = 0 AND batch_id > 0),
                    EXISTS (SELECT 1 FROM events WHERE undone = 1)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Drops history older than `before_ms` (keeps the redo stack).
    pub fn compact_events(&self, before_ms: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM events WHERE at < ?1 AND undone = 0",
            [before_ms],
        )?)
    }
}

fn outcome(batch: i64, events: &[EventRow]) -> UndoOutcome {
    let mut kinds = Vec::new();
    let mut titles = Vec::new();
    for (kind, before, after) in events {
        kinds.push(kind.clone());
        let title = after
            .as_deref()
            .or(before.as_deref())
            .and_then(|j| serde_json::from_str::<TaskSnapshot>(j).ok())
            .map(|s| s.title);
        if let Some(title) = title {
            if !titles.contains(&title) {
                titles.push(title);
            }
        }
    }
    UndoOutcome {
        batch,
        kinds,
        titles,
    }
}

const IGNORED_FIELDS: [&str; 2] = ["updated_at", "sort_order"];

fn changed_fields(before: &str, after: &str) -> Result<Vec<String>> {
    let before: Value = serde_json::from_str(before)?;
    let after: Value = serde_json::from_str(after)?;
    let (Value::Object(b), Value::Object(a)) = (before, after) else {
        return Ok(Vec::new());
    };
    Ok(a.iter()
        .filter(|(k, v)| !IGNORED_FIELDS.contains(&k.as_str()) && b.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect())
}
