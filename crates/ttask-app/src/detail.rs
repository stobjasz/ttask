use serde::Serialize;
use ttask_core::{ChecklistItem, Task, TaskId};
use ttask_store::{Db, Reminder, TimeEntry};

use crate::Result;

/// Everything `ttask show` and the TUI detail pane display.
#[derive(Debug, Clone, Serialize)]
pub struct TaskDetail {
    pub task: Task,
    pub parent: Option<Task>,
    pub subtasks: Vec<Task>,
    pub checklist: Vec<ChecklistItem>,
    pub reminders: Vec<Reminder>,
    /// Tasks this one waits on.
    pub blocked_by: Vec<Task>,
    /// Tasks waiting on this one.
    pub blocking: Vec<Task>,
    pub time_entries: Vec<TimeEntry>,
    pub tracked_minutes: i64,
}

impl TaskDetail {
    pub(crate) fn load(db: &Db<'_>, id: TaskId, now_ms: i64) -> Result<TaskDetail> {
        let task = db.task(id)?;
        let parent = task
            .parent_id
            .map(|p| db.try_task(p))
            .transpose()?
            .flatten();
        let load_all = |ids: Vec<TaskId>| -> Result<Vec<Task>> {
            Ok(ids
                .into_iter()
                .map(|i| db.try_task(i))
                .collect::<ttask_store::Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect())
        };
        let time_entries = db.time_entries(id)?;
        let tracked_minutes = time_entries.iter().map(|e| e.minutes(now_ms)).sum();
        Ok(TaskDetail {
            parent,
            subtasks: db.subtasks(id)?,
            checklist: db.checklist(id)?,
            reminders: db.reminders_for(id)?,
            blocked_by: load_all(db.blockers(id)?)?,
            blocking: load_all(db.dependents(id)?)?,
            time_entries,
            tracked_minutes,
            task,
        })
    }
}
