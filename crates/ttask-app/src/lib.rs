//! Application services: the single entry point both the CLI and the TUI use
//! to read and change data. Every mutating call is one transaction and one
//! undoable batch in the journal.

mod backups;
mod detail;
pub mod editor;
pub mod io;
pub mod launch;
mod reports;
mod templates;
mod views;

use jiff::ToSpan;
use jiff::civil::{Date, Time};
use ttask_core::filter::{Expr, FilterError, Pred, parse_filter};
use ttask_core::quickadd::{Change, DEFAULT_REMINDER_TIME, Mods, QuickAddError, Wait, parse_mods};
use ttask_core::recur::{RecurError, RecurMode, RecurRule, next_due};
use ttask_core::urgency::{UrgencyWeights, urgency};
use ttask_core::{
    ChecklistItem, Clock, NewTask, Priority, Project, ProjectId, SavedFilter, Status, TagInfo,
    Task, TaskId, jiff,
};
use ttask_store::{
    Db, EventKind, HistoryEntry, QueryCtx, SearchHit, Store, StoreError, TaskFields, TaskQuery,
    TaskSort, UndoOutcome,
};

pub use backups::{BackupPolicy, RotationReport};
pub use detail::TaskDetail;
pub use reports::{ProjectTime, Report, WeekCount};
pub use templates::{Template, TemplateKind, TemplateTask, UseTemplate};
pub use ttask_store::{Reminder, TimeEntry};
pub use views::{View, ViewKind, arrange_tree};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("a task needs a title")]
    EmptyTitle,

    #[error(transparent)]
    QuickAdd(#[from] QuickAddError),

    #[error(transparent)]
    Filter(#[from] FilterError),

    #[error(transparent)]
    Recur(#[from] RecurError),

    #[error("task {0} does not repeat")]
    NotRecurring(TaskId),

    #[error("{0}")]
    Invalid(String),

    /// Internal: rolls back a dry-run import.
    #[error("dry run")]
    DryRun,

    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

/// Result of completing one task.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Completion {
    pub task: Task,
    /// The next instance of a recurring task.
    pub next: Option<Task>,
    /// Open subtasks completed along with it.
    pub subtasks: Vec<Task>,
}

pub struct App {
    store: Store,
    clock: Box<dyn Clock>,
    urgency: UrgencyWeights,
}

/// The `meta` key holding the active context's saved-filter name.
const CONTEXT_KEY: &str = "context";
/// The `meta` key holding the date of the last finished weekly review.
const REVIEW_KEY: &str = "last_review";

impl App {
    pub fn new(store: Store, clock: impl Clock + 'static) -> App {
        App {
            store,
            clock: Box::new(clock),
            urgency: UrgencyWeights::default(),
        }
    }

    /// Replaces the urgency weights (`[urgency]` in the config).
    pub fn set_urgency_weights(&mut self, weights: UrgencyWeights) {
        self.urgency = weights;
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    /// Gives the store back (e.g. to change encryption, which consumes it).
    pub fn into_store(self) -> Store {
        self.store
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    pub fn today(&self) -> Date {
        self.clock.today()
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    pub fn ctx(&self) -> QueryCtx {
        QueryCtx {
            today: self.today(),
            tz: self.clock.time_zone(),
        }
    }

    // ---------------------------------------------------------------- reads

    pub fn task(&self, id: TaskId) -> Result<Task> {
        let task = self.store.db().task(id)?;
        let active = self.store.db().running_timer()?.map(|e| e.task_id);
        Ok(self.with_urgency(task, active))
    }

    /// Tasks matching `query` (plus `extra`, ANDed) within the active
    /// context. Without a status term only open tasks are returned; without
    /// a `wait:`/`is:waiting`/`is:someday` term, waiting and someday tasks
    /// are hidden.
    pub fn list(&self, query: &str, extra: &str, sort: TaskSort) -> Result<Vec<Task>> {
        self.list_with(query, extra, sort, true)
    }

    /// [`App::list`], optionally ignoring the active context.
    pub fn list_with(
        &self,
        query: &str,
        extra: &str,
        sort: TaskSort,
        use_context: bool,
    ) -> Result<Vec<Task>> {
        let filter = self.effective_filter(query, extra, use_context)?;
        let query = TaskQuery {
            filter,
            sort,
            limit: None,
        };
        let tasks = self.store.db().query_tasks(&query, &self.ctx())?;
        let active = self.store.db().running_timer()?.map(|e| e.task_id);
        let mut tasks: Vec<Task> = tasks
            .into_iter()
            .map(|t| self.with_urgency(t, active))
            .collect();
        if sort == TaskSort::Urgency {
            // Stable: equal urgency keeps the SQL order (due date, priority).
            tasks.sort_by(|a, b| b.urgency.total_cmp(&a.urgency));
        }
        Ok(tasks)
    }

    fn effective_filter(&self, query: &str, extra: &str, use_context: bool) -> Result<Expr> {
        let today = self.today();
        let mut expr = parse_filter(query, today)?.and(parse_filter(extra, today)?);
        if !expr.constrains_status() {
            expr = expr.and(Expr::Pred(Pred::Status(Status::Open)));
        }
        if !expr.constrains_visibility() {
            expr = expr
                .and(Expr::Not(Box::new(Expr::Pred(Pred::IsWaiting))))
                .and(Expr::Not(Box::new(Expr::Pred(Pred::IsSomeday))));
        }
        if use_context {
            if let Some((_, context)) = self.active_context()? {
                expr = expr.and(context);
            }
        }
        Ok(expr)
    }

    fn with_urgency(&self, mut task: Task, active: Option<TaskId>) -> Task {
        task.urgency = urgency(
            &task,
            self.today(),
            self.now_ms(),
            active == Some(task.id),
            &self.urgency,
        );
        task
    }

    pub fn view_tasks(&self, view: &View, extra: &str) -> Result<Vec<Task>> {
        self.list(&view.query, extra, view.sort)
    }

    /// Task count for a view within the active context (sidebar badges).
    pub fn view_count(&self, view: &View) -> Result<u32> {
        let expr = self.effective_filter(&view.query, "", true)?;
        Ok(self.store.db().count_tasks(&expr, &self.ctx())?)
    }

    // ------------------------------------------------------------- contexts

    /// The active context: a saved filter applied to every list. A context
    /// whose saved filter was deleted counts as none.
    pub fn active_context(&self) -> Result<Option<(String, Expr)>> {
        let db = self.store.db();
        let Some(name) = db.meta(CONTEXT_KEY)? else {
            return Ok(None);
        };
        let Some(filter) = db.saved_filters()?.into_iter().find(|f| f.name == name) else {
            return Ok(None);
        };
        Ok(Some((
            filter.name,
            parse_filter(&filter.query, self.today())?,
        )))
    }

    /// Activates the saved filter `name` as the context, or clears it.
    pub fn set_context(&mut self, name: Option<&str>) -> Result<Option<String>> {
        let name = match name {
            None => None,
            Some(wanted) => Some(
                self.saved_filters()?
                    .into_iter()
                    .find(|f| f.name.eq_ignore_ascii_case(wanted))
                    .map(|f| f.name)
                    .ok_or_else(|| StoreError::NotFound(format!("saved filter `{wanted}`")))?,
            ),
        };
        self.store
            .write(|db| db.set_meta(CONTEXT_KEY, name.as_deref()))?;
        Ok(name)
    }

    /// Saves `query` as a filter called `name` and makes it the context.
    pub fn define_context(&mut self, name: &str, query: &str) -> Result<()> {
        self.save_filter(name, query)?;
        self.set_context(Some(name))?;
        Ok(())
    }

    /// Project and tags a new task inherits from the context, when the
    /// context is a plain `#project` and/or `+tag` combination.
    fn context_defaults(&self) -> Result<(Option<Vec<String>>, Vec<String>)> {
        let Some((_, expr)) = self.active_context()? else {
            return Ok((None, Vec::new()));
        };
        let terms: Vec<&Expr> = match &expr {
            Expr::And(items) => items.iter().collect(),
            single => vec![single],
        };
        let mut project = None;
        let mut tags = Vec::new();
        for term in terms {
            match term {
                Expr::Pred(Pred::Project(path)) if project.is_none() => {
                    project = Some(path.split('/').map(str::to_owned).collect());
                }
                Expr::Pred(Pred::Tag(tag)) => tags.push(tag.clone()),
                // Anything else (or, not, dates…) cannot be turned into defaults.
                _ => return Ok((None, Vec::new())),
            }
        }
        Ok((project, tags))
    }

    pub fn search(&self, text: &str, include_closed: bool) -> Result<Vec<SearchHit>> {
        Ok(self.store.db().search(text, include_closed, 200)?)
    }

    pub fn projects(&self, include_archived: bool) -> Result<Vec<Project>> {
        Ok(self.store.db().projects(include_archived)?)
    }

    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        Ok(self.store.db().tags()?)
    }

    pub fn saved_filters(&self) -> Result<Vec<SavedFilter>> {
        Ok(self.store.db().saved_filters()?)
    }

    pub fn history(&self, id: TaskId) -> Result<Vec<HistoryEntry>> {
        Ok(self.store.db().history(id)?)
    }

    pub fn detail(&self, id: TaskId) -> Result<TaskDetail> {
        let mut detail = TaskDetail::load(&self.store.db(), id, self.now_ms())?;
        let active = self.store.db().running_timer()?.map(|e| e.task_id);
        detail.task = self.with_urgency(detail.task, active);
        Ok(detail)
    }

    /// Openable links in a task's title, notes and checklist, in that order.
    pub fn links(&self, id: TaskId) -> Result<Vec<String>> {
        let detail = TaskDetail::load(&self.store.db(), id, self.now_ms())?;
        let mut text = format!("{}\n{}", detail.task.title, detail.task.notes);
        for item in &detail.checklist {
            text.push('\n');
            text.push_str(&item.text);
        }
        Ok(ttask_core::links::find_links(&text))
    }

    /// Active projects with no open task you could start now (not
    /// waiting, someday or blocked), for the weekly review.
    pub fn projects_without_next_action(&self) -> Result<Vec<Project>> {
        let mut out = Vec::new();
        for project in self.projects(false)? {
            let query = format!("project:\"{}\" and not is:blocked", project.path);
            if self.count(&query, "", false)? == 0 {
                out.push(project);
            }
        }
        Ok(out)
    }

    /// The day the weekly review was last finished.
    pub fn last_review(&self) -> Result<Option<Date>> {
        Ok(self
            .store
            .db()
            .meta(REVIEW_KEY)?
            .and_then(|d| d.parse().ok()))
    }

    pub fn mark_reviewed(&mut self) -> Result<()> {
        let today = self.today().to_string();
        Ok(self
            .store
            .write(|db| db.set_meta(REVIEW_KEY, Some(&today)))?)
    }

    /// How many tasks [`App::list_with`] would return.
    pub fn count(&self, query: &str, extra: &str, use_context: bool) -> Result<u32> {
        let expr = self.effective_filter(query, extra, use_context)?;
        Ok(self.store.db().count_tasks(&expr, &self.ctx())?)
    }

    /// `(can_undo, can_redo)`.
    pub fn undo_state(&self) -> Result<(bool, bool)> {
        Ok(self.store.db().undo_state()?)
    }

    // ------------------------------------------------------------ mutations

    /// Creates a task from quick-add text, e.g. `Pay rent #home !high due:fri`.
    /// `project` and `parent` are defaults the text can override.
    pub fn add(
        &mut self,
        text: &str,
        project: Option<ProjectId>,
        parent: Option<TaskId>,
    ) -> Result<Task> {
        let mods = parse_mods(text, self.today())?;
        self.add_with(&mods, project, parent)
    }

    pub fn add_with(
        &mut self,
        mods: &Mods,
        project: Option<ProjectId>,
        parent: Option<TaskId>,
    ) -> Result<Task> {
        let title = mods.title.clone().unwrap_or_default();
        if title.trim().is_empty() {
            return Err(AppError::EmptyTitle);
        }
        // Inside a context like `#work +office`, new tasks land there too.
        let mut with_defaults;
        let mut mods = mods;
        if project.is_none() && parent.is_none() {
            let (ctx_project, ctx_tags) = self.context_defaults()?;
            if ctx_project.is_some() || !ctx_tags.is_empty() {
                with_defaults = mods.clone();
                if with_defaults.project.is_none() {
                    with_defaults.project = ctx_project.map(Change::Set);
                }
                with_defaults.add_tags.extend(ctx_tags);
                mods = &with_defaults;
            }
        }
        let (today, tz, now) = (self.today(), self.clock.time_zone(), self.now_ms());
        self.batch(|tx| {
            let mut project_id = project;
            if let Some(parent) = parent {
                // Subtasks live in their parent's project unless told otherwise.
                project_id = tx.db.task(parent)?.project_id;
            }
            let draft = NewTask {
                title,
                project_id,
                parent_id: parent,
                ..NewTask::default()
            };
            let task = tx.db.insert_task(&draft, now)?;
            let mut fields = TaskFields::from(&task);
            apply_mods(tx.db, &mut fields, mods, today, now)?;
            let task = tx.db.save_task(task.id, &fields, now)?;
            set_reminders(tx.db, &task, mods, today, &tz)?;
            tx.record_new(task.id, EventKind::Created)?;
            Ok(tx.db.task(task.id)?)
        })
    }

    /// Applies edit text (`due:+3d -+later #none New title`) to each task.
    pub fn edit(&mut self, ids: &[TaskId], text: &str) -> Result<Vec<Task>> {
        let mods = parse_mods(text, self.today())?;
        if mods.is_empty() {
            return Err(AppError::Invalid("nothing to change".into()));
        }
        self.edit_with(ids, &mods)
    }

    pub fn edit_with(&mut self, ids: &[TaskId], mods: &Mods) -> Result<Vec<Task>> {
        let (today, tz, now) = (self.today(), self.clock.time_zone(), self.now_ms());
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                tx.track(id, EventKind::Updated, |db| {
                    let task = db.task(id)?;
                    let mut fields = TaskFields::from(&task);
                    apply_mods(db, &mut fields, mods, today, now)?;
                    let task = db.save_task(id, &fields, now)?;
                    set_reminders(db, &task, mods, today, &tz)?;
                    Ok(())
                })?;
                out.push(tx.db.task(id)?);
            }
            Ok(out)
        })
    }

    pub fn set_title(&mut self, id: TaskId, title: &str) -> Result<Task> {
        if title.trim().is_empty() {
            return Err(AppError::EmptyTitle);
        }
        self.update_fields(id, |f| f.title = title.trim().to_owned())
    }

    pub fn set_notes(&mut self, id: TaskId, notes: &str) -> Result<Task> {
        self.update_fields(id, |f| f.notes = notes.trim_end().to_owned())
    }

    pub fn set_priority(&mut self, ids: &[TaskId], priority: Priority) -> Result<Vec<Task>> {
        let mods = Mods {
            priority: Some(priority),
            ..Mods::default()
        };
        self.edit_with(ids, &mods)
    }

    /// Makes `child` a subtask of `parent`, or a top-level task with `None`.
    pub fn set_parent(&mut self, child: TaskId, parent: Option<TaskId>) -> Result<Task> {
        self.update_fields(child, |f| f.parent_id = parent)
    }

    /// Appends a timestamped line to the notes: `- 2026-10-16 14:02 text`.
    pub fn annotate(&mut self, id: TaskId, text: &str) -> Result<Task> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AppError::Invalid("an annotation needs text".into()));
        }
        let stamp = jiff::Timestamp::from_millisecond(self.now_ms())
            .map(|t| {
                t.to_zoned(self.clock.time_zone())
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let line = format!("- {stamp} {text}");
        self.update_fields(id, |f| {
            let notes = f.notes.trim_end();
            f.notes = if notes.is_empty() {
                line
            } else {
                format!(
                    "{notes}
{line}"
                )
            };
        })
    }

    fn update_fields(&mut self, id: TaskId, change: impl FnOnce(&mut TaskFields)) -> Result<Task> {
        let now = self.now_ms();
        self.batch(|tx| {
            tx.track(id, EventKind::Updated, |db| {
                let mut fields = TaskFields::from(&db.task(id)?);
                change(&mut fields);
                db.save_task(id, &fields, now)?;
                Ok(())
            })?;
            Ok(tx.db.task(id)?)
        })
    }

    /// Marks tasks done. A recurring task spawns its next instance; open
    /// subtasks are completed too. Already-closed tasks are left as they are.
    pub fn complete(&mut self, ids: &[TaskId]) -> Result<Vec<Completion>> {
        let (today, tz, now) = (self.today(), self.clock.time_zone(), self.now_ms());
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                let task = tx.db.task(id)?;
                if !task.is_open() {
                    out.push(Completion {
                        task,
                        next: None,
                        subtasks: Vec::new(),
                    });
                    continue;
                }
                let mut subtasks = Vec::new();
                for child in tx.db.subtasks(id)?.into_iter().filter(Task::is_open) {
                    close(tx, child.id, Status::Done, now)?;
                    subtasks.push(tx.db.task(child.id)?);
                }
                close(tx, id, Status::Done, now)?;
                let next = spawn_next(tx, &task, today, &tz, now)?;
                out.push(Completion {
                    task: tx.db.task(id)?,
                    next,
                    subtasks,
                });
            }
            Ok(out)
        })
    }

    pub fn cancel(&mut self, ids: &[TaskId]) -> Result<Vec<Task>> {
        let now = self.now_ms();
        self.batch(|tx| {
            ids.iter()
                .map(|&id| {
                    close(tx, id, Status::Cancelled, now)?;
                    Ok(tx.db.task(id)?)
                })
                .collect()
        })
    }

    pub fn reopen(&mut self, ids: &[TaskId]) -> Result<Vec<Task>> {
        let now = self.now_ms();
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                tx.track(id, EventKind::Reopened, |db| {
                    let mut fields = TaskFields::from(&db.task(id)?);
                    fields.status = Status::Open;
                    fields.completed_at = None;
                    db.save_task(id, &fields, now)?;
                    Ok(())
                })?;
                out.push(tx.db.task(id)?);
            }
            Ok(out)
        })
    }

    /// Toggles done/open (the TUI's `x`).
    pub fn toggle_done(&mut self, id: TaskId) -> Result<Completion> {
        let task = self.task(id)?;
        if task.is_open() {
            Ok(self.complete(&[id])?.remove(0))
        } else {
            Ok(Completion {
                task: self.reopen(&[id])?.remove(0),
                next: None,
                subtasks: Vec::new(),
            })
        }
    }

    /// Soft-deletes tasks and their subtasks.
    pub fn delete(&mut self, ids: &[TaskId]) -> Result<Vec<Task>> {
        let now = self.now_ms();
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                let Some(task) = tx.db.try_task(id)? else {
                    // Already gone with a parent deleted earlier in this batch.
                    if tx.db.snapshot(id)?.is_some() {
                        continue;
                    }
                    return Err(StoreError::TaskNotFound(id).into());
                };
                for child in tx.db.subtasks(id)? {
                    tx.track(child.id, EventKind::Deleted, |db| {
                        db.soft_delete_task(child.id, now)
                    })?;
                }
                tx.track(id, EventKind::Deleted, |db| db.soft_delete_task(id, now))?;
                out.push(task);
            }
            Ok(out)
        })
    }

    /// Moves due dates by `days` (one undo step), keeping the time. A task
    /// without a due date gets today + `days` when that is not in the past.
    pub fn shift_due(&mut self, ids: &[TaskId], days: i64) -> Result<Vec<Task>> {
        let (today, now) = (self.today(), self.now_ms());
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                tx.track(id, EventKind::Updated, |db| {
                    let mut fields = TaskFields::from(&db.task(id)?);
                    fields.due_date = match fields.due_date {
                        Some(due) => Some(due + days.days()),
                        None if days > 0 => Some(today + days.days()),
                        None => None,
                    };
                    db.save_task(id, &fields, now)?;
                    Ok(())
                })?;
                out.push(tx.db.task(id)?);
            }
            Ok(out)
        })
    }

    /// Moves a recurring task to its next occurrence without completing it.
    pub fn skip(&mut self, id: TaskId) -> Result<Task> {
        let (today, now) = (self.today(), self.now_ms());
        self.batch(|tx| {
            let task = tx.db.task(id)?;
            let (rule, mode) = recurrence(&task)?.ok_or(AppError::NotRecurring(id))?;
            let next = next_due(&rule, mode, task.due_date, today).ok_or_else(|| {
                AppError::Invalid(format!("task {id} has no further occurrences"))
            })?;
            tx.track(id, EventKind::Updated, |db| {
                let mut fields = TaskFields::from(&task);
                fields.scheduled_date = shift(task.scheduled_date, task.due_date, next);
                fields.wait_until = shift(task.wait_until, task.due_date, next);
                fields.due_date = Some(next);
                if let Some(advanced) = rule.advanced() {
                    fields.recur_rule = Some(advanced.to_rrule());
                }
                db.save_task(id, &fields, now)?;
                Ok(())
            })?;
            Ok(tx.db.task(id)?)
        })
    }

    /// Swaps manual order with the neighbour in `order` (the list as shown).
    pub fn reorder(&mut self, id: TaskId, order: &[TaskId], up: bool) -> Result<()> {
        let Some(pos) = order.iter().position(|&t| t == id) else {
            return Ok(());
        };
        let neighbour = if up {
            pos.checked_sub(1)
        } else {
            Some(pos + 1)
        };
        let Some(&other) = neighbour.and_then(|p| order.get(p)) else {
            return Ok(());
        };
        self.batch(|tx| {
            let a = tx.db.task(id)?;
            let b = tx.db.task(other)?;
            let (mut sa, mut sb) = (b.sort_order, a.sort_order);
            if sa == sb {
                // Equal keys cannot be swapped; nudge apart.
                if up {
                    sa -= 0.5;
                } else {
                    sa += 0.5;
                }
                sb = a.sort_order;
            }
            for (task, order) in [(id, sa), (other, sb)] {
                tx.track(task, EventKind::Updated, |db| {
                    db.set_sort_order(task, order)
                })?;
            }
            Ok(())
        })
    }

    pub fn undo(&mut self) -> Result<Option<UndoOutcome>> {
        Ok(self.store.write(|db| db.undo())?)
    }

    pub fn redo(&mut self) -> Result<Option<UndoOutcome>> {
        Ok(self.store.write(|db| db.redo())?)
    }

    // ------------------------------------------------------------ checklist

    pub fn checklist_add(&mut self, task: TaskId, text: &str) -> Result<ChecklistItem> {
        self.batch(|tx| {
            tx.track(task, EventKind::Updated, |db| {
                db.add_checklist_item(task, text)
            })
        })
    }

    pub fn checklist_toggle(&mut self, item: i64) -> Result<bool> {
        self.batch(|tx| {
            let task = tx.db.checklist_item_task(item)?;
            tx.track(task, EventKind::Updated, |db| {
                let current = db
                    .checklist(task)?
                    .into_iter()
                    .find(|i| i.id == item)
                    .map(|i| i.checked)
                    .unwrap_or(false);
                db.set_checklist_checked(item, !current)?;
                Ok(!current)
            })
        })
    }

    pub fn checklist_edit(&mut self, item: i64, text: &str) -> Result<()> {
        self.batch(|tx| {
            let task = tx.db.checklist_item_task(item)?;
            tx.track(task, EventKind::Updated, |db| {
                db.edit_checklist_item(item, text)
            })
        })
    }

    pub fn checklist_remove(&mut self, item: i64) -> Result<()> {
        self.batch(|tx| {
            let task = tx.db.checklist_item_task(item)?;
            tx.track(task, EventKind::Updated, |db| {
                db.remove_checklist_item(item)
            })
        })
    }

    // --------------------------------------------------------- dependencies

    pub fn block(&mut self, blocked: TaskId, blocker: TaskId) -> Result<()> {
        self.batch(|tx| {
            tx.track(blocked, EventKind::Updated, |db| {
                db.add_dependency(blocker, blocked)
            })
        })
    }

    pub fn unblock(&mut self, blocked: TaskId, blocker: TaskId) -> Result<()> {
        self.batch(|tx| {
            tx.track(blocked, EventKind::Updated, |db| {
                db.remove_dependency(blocker, blocked)
            })
        })
    }

    // ------------------------------------------------------------- time log

    pub fn start_timer(&mut self, task: TaskId) -> Result<(Option<TimeEntry>, TimeEntry)> {
        let now = self.now_ms();
        Ok(self.store.write(|db| db.start_timer(task, now))?)
    }

    pub fn stop_timer(&mut self) -> Result<Option<TimeEntry>> {
        let now = self.now_ms();
        Ok(self.store.write(|db| db.stop_timer(now))?)
    }

    pub fn running_timer(&self) -> Result<Option<(TimeEntry, Task)>> {
        let db = self.store.db();
        Ok(match db.running_timer()? {
            Some(entry) => {
                let task = db.task(entry.task_id)?;
                Some((entry, task))
            }
            None => None,
        })
    }

    // ------------------------------------------------------------ reminders

    /// Reminders due now; marks them fired and returns them with their task.
    pub fn take_due_reminders(&mut self) -> Result<Vec<(Reminder, Task)>> {
        let now = self.now_ms();
        Ok(self.store.write(|db| {
            let mut out = Vec::new();
            for reminder in db.due_reminders(now)? {
                db.mark_reminder_fired(reminder.id, now)?;
                out.push((reminder.clone(), db.task(reminder.task_id)?));
            }
            Ok(out)
        })?)
    }

    pub fn upcoming_reminders(&self, limit: u32) -> Result<Vec<(Reminder, Task)>> {
        let db = self.store.db();
        db.upcoming_reminders(self.now_ms(), limit)?
            .into_iter()
            .map(|r| {
                let task = db.task(r.task_id)?;
                Ok((r, task))
            })
            .collect()
    }

    // ------------------------------------------------ projects, tags, filters

    pub fn ensure_project(&mut self, path: &str) -> Result<ProjectId> {
        let segments: Vec<String> = path.split('/').map(|s| s.trim().to_owned()).collect();
        let now = self.now_ms();
        Ok(self
            .store
            .write(|db| db.ensure_project_path(&segments, now))?)
    }

    pub fn find_project(&self, path: &str) -> Result<Project> {
        self.store
            .db()
            .find_project(path)?
            .ok_or_else(|| StoreError::NotFound(format!("project `{path}`")).into())
    }

    pub fn rename_project(&mut self, path: &str, new_name: &str) -> Result<()> {
        let id = self.find_project(path)?.id;
        let now = self.now_ms();
        Ok(self
            .store
            .write(|db| db.rename_project(id, new_name, now))?)
    }

    pub fn archive_project(&mut self, path: &str, archived: bool) -> Result<()> {
        let id = self.find_project(path)?.id;
        let now = self.now_ms();
        Ok(self
            .store
            .write(|db| db.set_project_archived(id, archived, now))?)
    }

    pub fn delete_project(&mut self, path: &str) -> Result<usize> {
        let id = self.find_project(path)?.id;
        let now = self.now_ms();
        Ok(self.store.write(|db| db.delete_project(id, now))?)
    }

    pub fn rename_tag(&mut self, old: &str, new: &str) -> Result<()> {
        Ok(self.store.write(|db| db.rename_tag(old, new))?)
    }

    pub fn delete_tag(&mut self, name: &str) -> Result<()> {
        Ok(self.store.write(|db| db.delete_tag(name))?)
    }

    pub fn save_filter(&mut self, name: &str, query: &str) -> Result<()> {
        // Validate before storing.
        parse_filter(query, self.today())?;
        Ok(self.store.write(|db| db.save_filter(name, query))?)
    }

    pub fn delete_filter(&mut self, name: &str) -> Result<()> {
        Ok(self.store.write(|db| db.delete_filter(name))?)
    }

    // ------------------------------------------------------------- plumbing

    /// Runs `f` as one transaction and one undo batch.
    fn batch<T>(&mut self, f: impl FnOnce(&Batch<'_, '_>) -> Result<T>) -> Result<T> {
        let now = self.now_ms();
        self.store.write_with(|db| {
            let batch = Batch {
                db,
                id: db.begin_batch()?,
                now,
            };
            f(&batch)
        })
    }
}

/// One undoable batch inside a write transaction.
struct Batch<'a, 'b> {
    db: &'a Db<'b>,
    id: i64,
    now: i64,
}

impl Batch<'_, '_> {
    /// Snapshots `task` around `f` and journals the change, if any.
    fn track<T>(
        &self,
        task: TaskId,
        kind: EventKind,
        f: impl FnOnce(&Db<'_>) -> ttask_store::Result<T>,
    ) -> Result<T> {
        let before = self.db.snapshot(task)?;
        let out = f(self.db)?;
        let after = self.db.snapshot(task)?;
        if before != after {
            self.db.record_event(
                self.id,
                task,
                kind,
                before.as_ref(),
                after.as_ref(),
                self.now,
            )?;
        }
        Ok(out)
    }

    /// Journals a task created in this batch.
    fn record_new(&self, task: TaskId, kind: EventKind) -> Result<()> {
        let after = self.db.snapshot(task)?;
        self.db
            .record_event(self.id, task, kind, None, after.as_ref(), self.now)?;
        Ok(())
    }
}

fn close(tx: &Batch<'_, '_>, id: TaskId, status: Status, now: i64) -> Result<()> {
    let kind = if status == Status::Done {
        EventKind::Completed
    } else {
        EventKind::Updated
    };
    tx.track(id, kind, |db| {
        let mut fields = TaskFields::from(&db.task(id)?);
        fields.status = status;
        fields.completed_at = Some(now);
        db.save_task(id, &fields, now)?;
        Ok(())
    })
}

fn recurrence(task: &Task) -> Result<Option<(RecurRule, RecurMode)>> {
    match &task.recur_rule {
        Some(rule) => Ok(Some((rule.parse()?, task.recur_mode.unwrap_or_default()))),
        None => Ok(None),
    }
}

/// Creates the next instance of a just-completed recurring task.
fn spawn_next(
    tx: &Batch<'_, '_>,
    task: &Task,
    today: Date,
    tz: &jiff::tz::TimeZone,
    now: i64,
) -> Result<Option<Task>> {
    let Some((rule, mode)) = recurrence(task)? else {
        return Ok(None);
    };
    let Some(next_rule) = rule.advanced() else {
        return Ok(None);
    };
    let Some(next) = next_due(&rule, mode, task.due_date, today) else {
        return Ok(None);
    };
    let new = NewTask {
        title: task.title.clone(),
        notes: task.notes.clone(),
        priority: task.priority,
        project_id: task.project_id,
        parent_id: task.parent_id,
        due_date: Some(next),
        due_time: task.due_time,
        scheduled_date: shift(task.scheduled_date, task.due_date, next),
        wait_until: shift(task.wait_until, task.due_date, next),
        someday: false,
        estimate_min: task.estimate_min,
        recur_rule: Some(next_rule.to_rrule()),
        recur_mode: Some(mode),
        recur_series: Some(
            task.recur_series
                .clone()
                .unwrap_or_else(|| task.uid.clone()),
        ),
        tags: task.tags.clone(),
    };
    let spawned = tx.db.insert_task(&new, now)?;
    for item in tx.db.checklist(task.id)? {
        tx.db.add_checklist_item(spawned.id, &item.text)?;
    }
    // Reminders keep their offset from the due date.
    if let Some(old_due) = task.due_date {
        let old_at = due_instant(old_due, task.due_time, tz);
        let new_at = due_instant(next, task.due_time, tz);
        for reminder in tx.db.reminders_for(task.id)? {
            tx.db
                .add_reminder(spawned.id, reminder.fire_at - old_at + new_at)?;
        }
    }
    tx.record_new(spawned.id, EventKind::Spawned)?;
    Ok(Some(tx.db.task(spawned.id)?))
}

/// Moves `date` by the same number of days `old_anchor` moved to `new_anchor`.
fn shift(date: Option<Date>, old_anchor: Option<Date>, new_anchor: Date) -> Option<Date> {
    let (date, old) = (date?, old_anchor?);
    let days = ttask_core::date::days_between(old, new_anchor);
    date.checked_add(days.days()).ok()
}

fn due_instant(date: Date, time: Option<Time>, tz: &jiff::tz::TimeZone) -> i64 {
    date.to_datetime(time.unwrap_or(DEFAULT_REMINDER_TIME))
        .to_zoned(tz.clone())
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or_default()
}

/// Applies parsed quick-add/edit modifications to a task's fields.
fn apply_mods(
    db: &Db<'_>,
    f: &mut TaskFields,
    mods: &Mods,
    today: Date,
    now: i64,
) -> ttask_store::Result<()> {
    if let Some(title) = &mods.title {
        f.title = title.clone();
    }
    if let Some(notes) = &mods.notes {
        f.notes = notes.trim_end().to_owned();
    }
    match &mods.project {
        Some(Change::Set(path)) => f.project_id = Some(db.ensure_project_path(path, now)?),
        Some(Change::Clear) => f.project_id = None,
        None => {}
    }
    for tag in &mods.add_tags {
        if !f.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            f.tags.push(tag.clone());
        }
    }
    f.tags
        .retain(|t| !mods.remove_tags.iter().any(|r| r.eq_ignore_ascii_case(t)));
    if let Some(priority) = mods.priority {
        f.priority = priority;
    }
    match &mods.due {
        Some(Change::Set(spec)) => {
            f.due_date = Some(spec.date);
            f.due_time = spec.time;
        }
        Some(Change::Clear) => {
            f.due_date = None;
            f.due_time = None;
        }
        None => {}
    }
    match &mods.wait {
        Some(Change::Set(Wait::Until(date))) => {
            f.wait_until = Some(*date);
            f.someday = false;
        }
        Some(Change::Set(Wait::Someday)) => {
            f.someday = true;
            f.wait_until = None;
        }
        Some(Change::Clear) => {
            f.wait_until = None;
            f.someday = false;
        }
        None => {}
    }
    match &mods.sched {
        Some(Change::Set(date)) => f.scheduled_date = Some(*date),
        Some(Change::Clear) => f.scheduled_date = None,
        None => {}
    }
    match &mods.estimate {
        Some(Change::Set(minutes)) => f.estimate_min = Some(*minutes),
        Some(Change::Clear) => f.estimate_min = None,
        None => {}
    }
    match &mods.recur {
        Some(Change::Set((rule, mode))) => {
            f.recur_rule = Some(rule.to_rrule());
            f.recur_mode = Some(*mode);
            if f.due_date.is_none() {
                // A repeating task needs a date: its first occurrence from today.
                let yesterday = today.yesterday().unwrap_or(today);
                f.due_date = rule.next_after(yesterday, today);
            }
        }
        Some(Change::Clear) => {
            f.recur_rule = None;
            f.recur_mode = None;
        }
        None => {}
    }
    Ok(())
}

fn set_reminders(
    db: &Db<'_>,
    task: &Task,
    mods: &Mods,
    today: Date,
    tz: &jiff::tz::TimeZone,
) -> ttask_store::Result<()> {
    if mods.clear_reminders {
        db.clear_reminders(task.id)?;
    }
    let due = task.due_date.map(|d| (d, task.due_time));
    for spec in &mods.reminders {
        let at = spec.resolve(due, today).ok_or_else(|| {
            StoreError::Invalid("a reminder relative to the due date needs a due date".into())
        })?;
        let ms = at
            .to_zoned(tz.clone())
            .map_err(|e| StoreError::Invalid(format!("reminder time: {e}")))?
            .timestamp()
            .as_millisecond();
        db.add_reminder(task.id, ms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
