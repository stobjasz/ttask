use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use ttask_core::jiff::civil::{Date, Time};
use ttask_core::recur::RecurMode;
use ttask_core::{NewTask, Priority, Progress, ProjectId, Status, Task, TaskId, new_uid};

use crate::db::{Db, conversion_error, date_sql, parse_date_col, parse_time_col, time_sql};
use crate::{Result, StoreError};

/// Columns selected for every [`Task`] load; `t` is the task row.
pub(crate) const TASK_SELECT: &str = "SELECT t.id, t.uid, t.parent_id, t.project_id,
       CASE WHEN pp.id IS NULL THEN p.name ELSE pp.name || '/' || p.name END AS project_path,
       t.title, t.notes, t.status, t.priority, t.due_date, t.due_time, t.scheduled_date,
       t.estimate_min, t.recur_rule, t.recur_mode, t.recur_series, t.sort_order,
       t.created_at, t.updated_at, t.completed_at, t.wait_until, t.someday,
       (SELECT group_concat(g.name, char(31)) FROM task_tags tt JOIN tags g ON g.id = tt.tag_id
         WHERE tt.task_id = t.id) AS tags,
       (SELECT count(*) FROM tasks c WHERE c.parent_id = t.id AND c.deleted_at IS NULL) AS sub_total,
       (SELECT count(*) FROM tasks c WHERE c.parent_id = t.id AND c.deleted_at IS NULL
          AND c.status <> 'open') AS sub_done,
       (SELECT count(*) FROM checklist_items ci WHERE ci.task_id = t.id) AS cl_total,
       (SELECT count(*) FROM checklist_items ci WHERE ci.task_id = t.id AND ci.checked) AS cl_done,
       EXISTS (SELECT 1 FROM task_deps d JOIN tasks b ON b.id = d.blocker_id
         WHERE d.blocked_id = t.id AND b.status = 'open' AND b.deleted_at IS NULL) AS blocked,
       EXISTS (SELECT 1 FROM task_deps d JOIN tasks w ON w.id = d.blocked_id
         WHERE d.blocker_id = t.id AND w.status = 'open' AND w.deleted_at IS NULL) AS blocking
  FROM tasks t
  LEFT JOIN projects p ON p.id = t.project_id
  LEFT JOIN projects pp ON pp.id = p.parent_id";

/// The editable part of a task. Services read a [`Task`], change these
/// fields and save them back in one call.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFields {
    pub title: String,
    pub notes: String,
    pub status: Status,
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
    pub sort_order: f64,
    pub completed_at: Option<i64>,
    pub tags: Vec<String>,
}

impl From<&Task> for TaskFields {
    fn from(t: &Task) -> Self {
        TaskFields {
            title: t.title.clone(),
            notes: t.notes.clone(),
            status: t.status,
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
            sort_order: t.sort_order,
            completed_at: t.completed_at,
            tags: t.tags.clone(),
        }
    }
}

impl Db<'_> {
    pub fn insert_task(&self, new: &NewTask, now_ms: i64) -> Result<Task> {
        self.insert_task_with_uid(new, &new_uid(), now_ms)
    }

    /// [`Db::insert_task`] with a given uid (imports).
    pub fn insert_task_with_uid(&self, new: &NewTask, uid: &str, now_ms: i64) -> Result<Task> {
        if new.title.trim().is_empty() {
            return Err(StoreError::Invalid("a task needs a title".into()));
        }
        if let Some(parent) = new.parent_id {
            self.check_parent(parent, None)?;
        }
        let sort_order: f64 = self.conn.query_row(
            "SELECT coalesce(max(sort_order), 0) + 1 FROM tasks",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO tasks (uid, project_id, parent_id, title, notes, priority, due_date,
                due_time, scheduled_date, estimate_min, recur_rule, recur_mode, recur_series,
                sort_order, created_at, updated_at, wait_until, someday)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16, ?17)",
            params![
                uid,
                new.project_id,
                new.parent_id,
                new.title.trim(),
                new.notes,
                new.priority.value(),
                date_sql(new.due_date),
                time_sql(new.due_time),
                date_sql(new.scheduled_date),
                new.estimate_min,
                new.recur_rule,
                new.recur_mode.map(RecurMode::as_str),
                new.recur_series,
                sort_order,
                now_ms,
                date_sql(new.wait_until),
                new.someday
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.set_tags(id, &new.tags)?;
        self.task(id)
    }

    pub fn task(&self, id: TaskId) -> Result<Task> {
        self.try_task(id)?.ok_or(StoreError::TaskNotFound(id))
    }

    pub fn try_task(&self, id: TaskId) -> Result<Option<Task>> {
        Ok(self
            .conn
            .query_row(
                &format!("{TASK_SELECT} WHERE t.id = ?1 AND t.deleted_at IS NULL"),
                [id],
                task_from_row,
            )
            .optional()?)
    }

    pub fn task_by_uid(&self, uid: &str) -> Result<Option<Task>> {
        Ok(self
            .conn
            .query_row(
                &format!("{TASK_SELECT} WHERE t.uid = ?1 AND t.deleted_at IS NULL"),
                [uid],
                task_from_row,
            )
            .optional()?)
    }

    /// Open and closed subtasks of `id`, in manual order.
    pub fn subtasks(&self, id: TaskId) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare(&format!(
            "{TASK_SELECT} WHERE t.parent_id = ?1 AND t.deleted_at IS NULL
             ORDER BY t.sort_order, t.id"
        ))?;
        let rows = stmt.query_map([id], task_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Writes every editable column and the tag set.
    pub fn save_task(&self, id: TaskId, f: &TaskFields, now_ms: i64) -> Result<Task> {
        if f.title.trim().is_empty() {
            return Err(StoreError::Invalid("a task needs a title".into()));
        }
        if let Some(parent) = f.parent_id {
            self.check_parent(parent, Some(id))?;
        }
        let changed = self.conn.execute(
            "UPDATE tasks SET title = ?2, notes = ?3, status = ?4, priority = ?5, project_id = ?6,
                parent_id = ?7, due_date = ?8, due_time = ?9, scheduled_date = ?10,
                estimate_min = ?11, recur_rule = ?12, recur_mode = ?13, recur_series = ?14,
                sort_order = ?15, completed_at = ?16, updated_at = ?17, wait_until = ?18,
                someday = ?19
             WHERE id = ?1 AND deleted_at IS NULL",
            params![
                id,
                f.title.trim(),
                f.notes,
                f.status.as_str(),
                f.priority.value(),
                f.project_id,
                f.parent_id,
                date_sql(f.due_date),
                time_sql(f.due_time),
                date_sql(f.scheduled_date),
                f.estimate_min,
                f.recur_rule,
                f.recur_mode.map(RecurMode::as_str),
                f.recur_series,
                f.sort_order,
                f.completed_at,
                now_ms,
                date_sql(f.wait_until),
                f.someday
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::TaskNotFound(id));
        }
        self.set_tags(id, &f.tags)?;
        self.task(id)
    }

    /// Soft-deletes a task and its subtasks.
    pub fn soft_delete_task(&self, id: TaskId, now_ms: i64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE tasks SET deleted_at = ?2, updated_at = ?2
             WHERE (id = ?1 OR parent_id = ?1) AND deleted_at IS NULL",
            params![id, now_ms],
        )?;
        if changed == 0 {
            return Err(StoreError::TaskNotFound(id));
        }
        Ok(())
    }

    /// Replaces the task's tags, creating tags that do not exist yet.
    pub fn set_tags(&self, id: TaskId, tags: &[String]) -> Result<()> {
        self.conn
            .execute("DELETE FROM task_tags WHERE task_id = ?1", [id])?;
        for tag in tags {
            let tag_id = self.ensure_tag(tag)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO task_tags (task_id, tag_id) VALUES (?1, ?2)",
                params![id, tag_id],
            )?;
        }
        Ok(())
    }

    pub fn set_sort_order(&self, id: TaskId, sort_order: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET sort_order = ?2 WHERE id = ?1",
            params![id, sort_order],
        )?;
        Ok(())
    }

    /// Subtasks are one level deep: a parent must be a live top-level task
    /// and not the task itself.
    fn check_parent(&self, parent: TaskId, child: Option<TaskId>) -> Result<()> {
        if Some(parent) == child {
            return Err(StoreError::Invalid(
                "a task cannot be its own subtask".into(),
            ));
        }
        let parent_task = self.task(parent)?;
        if parent_task.parent_id.is_some() {
            return Err(StoreError::Invalid(format!(
                "task {parent} is itself a subtask; subtasks nest one level"
            )));
        }
        if let Some(child) = child {
            let has_children: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM tasks WHERE parent_id = ?1 AND deleted_at IS NULL)",
                [child],
                |r| r.get(0),
            )?;
            if has_children {
                return Err(StoreError::Invalid(format!(
                    "task {child} has subtasks, so it cannot become one"
                )));
            }
        }
        Ok(())
    }
}

pub(crate) fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get("status")?;
    let status = status
        .parse::<Status>()
        .map_err(|e| conversion_error(7, Type::Text, e))?;
    let priority: u8 = row.get("priority")?;
    let priority = Priority::new(priority)
        .ok_or_else(|| conversion_error(8, Type::Integer, format!("priority {priority}")))?;
    let recur_mode: Option<String> = row.get("recur_mode")?;
    let recur_mode = recur_mode
        .map(|m| {
            m.parse::<RecurMode>()
                .map_err(|e| conversion_error(14, Type::Text, e))
        })
        .transpose()?;
    let tags: Option<String> = row.get("tags")?;
    let mut tags: Vec<String> = tags
        .map(|t| t.split('\u{1f}').map(str::to_owned).collect())
        .unwrap_or_default();
    tags.sort_by_key(|t| t.to_lowercase());
    Ok(Task {
        id: row.get("id")?,
        uid: row.get("uid")?,
        parent_id: row.get("parent_id")?,
        project_id: row.get("project_id")?,
        project: row.get("project_path")?,
        title: row.get("title")?,
        notes: row.get("notes")?,
        status,
        priority,
        due_date: parse_date_col(row.get("due_date")?, 9)?,
        due_time: parse_time_col(row.get("due_time")?, 10)?,
        scheduled_date: parse_date_col(row.get("scheduled_date")?, 11)?,
        estimate_min: row.get("estimate_min")?,
        recur_rule: row.get("recur_rule")?,
        recur_mode,
        recur_series: row.get("recur_series")?,
        wait_until: parse_date_col(row.get("wait_until")?, 20)?,
        someday: row.get("someday")?,
        tags,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        completed_at: row.get("completed_at")?,
        subtasks: Progress {
            done: row.get("sub_done")?,
            total: row.get("sub_total")?,
        },
        checklist: Progress {
            done: row.get("cl_done")?,
            total: row.get("cl_total")?,
        },
        blocked: row.get("blocked")?,
        blocking: row.get("blocking")?,
        urgency: 0.0,
    })
}
