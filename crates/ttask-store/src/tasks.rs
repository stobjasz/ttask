use rusqlite::types::{FromSqlError, Type};
use rusqlite::{OptionalExtension, Row, params};
use ttask_core::{NewTask, Priority, Status, Task, TaskId, new_uid};

use crate::{Result, Store, StoreError};

const TASK_COLUMNS: &str = "id, uid, title, notes, status, priority, due_date, due_time, \
                            created_at, updated_at, completed_at";

/// Which tasks a listing returns. Soft-deleted tasks are never listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskFilter {
    #[default]
    Open,
    All,
}

impl Store {
    pub fn insert_task(&mut self, new: &NewTask, now_ms: i64) -> Result<Task> {
        self.conn.execute(
            "INSERT INTO tasks (uid, title, notes, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                new_uid(),
                new.title,
                new.notes,
                new.priority.value(),
                now_ms
            ],
        )?;
        self.task(self.conn.last_insert_rowid())
    }

    pub fn task(&self, id: TaskId) -> Result<Task> {
        self.conn
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1 AND deleted_at IS NULL"),
                [id],
                task_from_row,
            )
            .optional()?
            .ok_or(StoreError::TaskNotFound(id))
    }

    /// Open tasks first by priority, then due date (undated last), then age.
    pub fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        let status_clause = match filter {
            TaskFilter::Open => "AND status = 'open'",
            TaskFilter::All => "",
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks
             WHERE deleted_at IS NULL {status_clause}
             ORDER BY status = 'open' DESC, priority DESC, due_date NULLS LAST, due_time NULLS LAST, id"
        ))?;
        let tasks = stmt
            .query_map([], task_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    /// Marks a task done. Completing an already-done task keeps its original
    /// completion time.
    pub fn complete_task(&mut self, id: TaskId, now_ms: i64) -> Result<Task> {
        self.conn.execute(
            "UPDATE tasks SET status = 'done', completed_at = ?2, updated_at = ?2
             WHERE id = ?1 AND deleted_at IS NULL AND status <> 'done'",
            params![id, now_ms],
        )?;
        // Zero rows changed means the task was already done (returned as is)
        // or is missing (reported by `task`).
        self.task(id)
    }
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get("status")?;
    let status = status
        .parse::<Status>()
        .map_err(|err| conversion_error(row, "status", Type::Text, err))?;
    let priority: u8 = row.get("priority")?;
    let priority = Priority::new(priority).ok_or_else(|| {
        let err = FromSqlError::OutOfRange(priority.into());
        conversion_error(row, "priority", Type::Integer, err)
    })?;
    Ok(Task {
        id: row.get("id")?,
        uid: row.get("uid")?,
        title: row.get("title")?,
        notes: row.get("notes")?,
        status,
        priority,
        due_date: row.get("due_date")?,
        due_time: row.get("due_time")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        completed_at: row.get("completed_at")?,
    })
}

fn conversion_error(
    row: &Row<'_>,
    column: &str,
    sql_type: Type,
    err: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    let index = row.as_ref().column_index(column).unwrap_or_default();
    rusqlite::Error::FromSqlConversionFailure(index, sql_type, Box::new(err))
}
