//! Raw access used by import/export: projects and history by uid, time
//! entries in bulk.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use ttask_core::{ProjectId, TaskId};

use crate::db::Db;
use crate::{Result, StoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub uid: String,
    pub name: String,
    pub parent_uid: Option<String>,
    pub color: Option<String>,
    pub sort_order: f64,
    pub archived_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub task_id: Option<TaskId>,
    pub at: i64,
    pub kind: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Db<'_> {
    /// Live projects (archived included), parents before children.
    pub fn project_records(&self) -> Result<Vec<ProjectRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.uid, p.name, pp.uid, p.color, p.sort_order, p.archived_at, p.created_at, p.updated_at
               FROM projects p LEFT JOIN projects pp ON pp.id = p.parent_id
              WHERE p.deleted_at IS NULL
              ORDER BY p.parent_id IS NOT NULL, p.sort_order, p.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProjectRecord {
                uid: r.get(0)?,
                name: r.get(1)?,
                parent_uid: r.get(2)?,
                color: r.get(3)?,
                sort_order: r.get(4)?,
                archived_at: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn project_id_by_uid(&self, uid: &str) -> Result<Option<ProjectId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM projects WHERE uid = ?1 AND deleted_at IS NULL",
                [uid],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Creates or updates a project identified by its uid. Returns
    /// `(id, created)`.
    pub fn upsert_project(
        &self,
        p: &ProjectRecord,
        parent: Option<ProjectId>,
    ) -> Result<(ProjectId, bool)> {
        if p.name.trim().is_empty() || p.name.contains('/') {
            return Err(StoreError::Invalid(format!(
                "invalid project name `{}`",
                p.name
            )));
        }
        if let Some(id) = self.project_id_by_uid(&p.uid)? {
            self.conn.execute(
                "UPDATE projects SET name = ?2, parent_id = ?3, color = ?4, sort_order = ?5,
                    archived_at = ?6, updated_at = ?7 WHERE id = ?1",
                params![
                    id,
                    p.name,
                    parent,
                    p.color,
                    p.sort_order,
                    p.archived_at,
                    p.updated_at
                ],
            )?;
            return Ok((id, false));
        }
        self.conn.execute(
            "INSERT INTO projects (uid, parent_id, name, color, sort_order, archived_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                p.uid,
                parent,
                p.name,
                p.color,
                p.sort_order,
                p.archived_at,
                p.created_at,
                p.updated_at
            ],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    /// Every live task id, including closed ones, in creation order.
    pub fn all_task_ids(&self) -> Result<Vec<TaskId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM tasks WHERE deleted_at IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn task_id_by_uid(&self, uid: &str) -> Result<Option<TaskId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM tasks WHERE uid = ?1 AND deleted_at IS NULL",
                [uid],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn task_uid(&self, id: TaskId) -> Result<String> {
        self.conn
            .query_row("SELECT uid FROM tasks WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or(StoreError::TaskNotFound(id))
    }

    /// The journal (undone entries excluded), oldest first.
    pub fn event_records(&self) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, at, kind, before, after FROM events WHERE undone = 0 ORDER BY at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(EventRecord {
                task_id: r.get(0)?,
                at: r.get(1)?,
                kind: r.get(2)?,
                before: r.get(3)?,
                after: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds imported history (batch 0: shown in history, never undone).
    pub fn insert_history(&self, task: TaskId, event: &EventRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (task_id, at, kind, before, after, batch_id) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![task, event.at, event.kind, event.before, event.after],
        )?;
        Ok(())
    }

    /// Replaces a task's time log.
    pub fn replace_time_entries(&self, task: TaskId, entries: &[(i64, Option<i64>)]) -> Result<()> {
        self.conn
            .execute("DELETE FROM time_entries WHERE task_id = ?1", [task])?;
        for (start, end) in entries {
            self.conn.execute(
                "INSERT INTO time_entries (task_id, started_at, ended_at) VALUES (?1, ?2, ?3)",
                params![task, start, end],
            )?;
        }
        Ok(())
    }
}
