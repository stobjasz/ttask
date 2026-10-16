//! Compiles the filter language to a parameterised SQL `WHERE` clause.
//! User text is always bound as a parameter, never spliced into SQL.

use rusqlite::params_from_iter;
use rusqlite::types::Value;
use ttask_core::filter::{Cmp, Expr, Pred};
use ttask_core::jiff::civil::Date;
use ttask_core::jiff::tz::TimeZone;
use ttask_core::{Task, TaskId, start_of_day_ms};

use crate::Result;
use crate::db::Db;
use crate::tasks::{TASK_SELECT, task_from_row};

/// What "today" and "midnight" mean for date predicates.
#[derive(Debug, Clone)]
pub struct QueryCtx {
    pub today: Date,
    pub tz: TimeZone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskSort {
    /// Open first, then by due date (undated last), priority, manual order.
    #[default]
    Smart,
    /// Manual order (`J`/`K` in the TUI), used inside a project.
    Manual,
    Priority,
    Created,
    /// Most recently completed first (logbook).
    Completed,
    Title,
    /// Most urgent first; the services layer computes urgency and re-sorts.
    Urgency,
}

impl TaskSort {
    fn order_by(self) -> &'static str {
        match self {
            TaskSort::Smart => {
                "(t.status = 'open') DESC, t.due_date IS NULL, t.due_date, t.due_time IS NULL,
                 t.due_time, t.priority DESC, t.sort_order, t.id"
            }
            TaskSort::Manual => "(t.status = 'open') DESC, t.sort_order, t.id",
            TaskSort::Priority => {
                "(t.status = 'open') DESC, t.priority DESC, t.due_date IS NULL, t.due_date, t.sort_order, t.id"
            }
            TaskSort::Created => "t.created_at DESC, t.id DESC",
            TaskSort::Completed => "t.completed_at IS NULL, t.completed_at DESC, t.id DESC",
            TaskSort::Title => "t.title COLLATE NOCASE, t.id",
            TaskSort::Urgency => {
                "(t.status = 'open') DESC, t.due_date IS NULL, t.due_date, t.priority DESC, t.id"
            }
        }
    }
}

impl std::str::FromStr for TaskSort {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "smart" | "due" => TaskSort::Smart,
            "manual" => TaskSort::Manual,
            "priority" => TaskSort::Priority,
            "created" => TaskSort::Created,
            "completed" | "done" => TaskSort::Completed,
            "title" => TaskSort::Title,
            "urgency" => TaskSort::Urgency,
            other => return Err(format!("unknown sort `{other}`")),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TaskQuery {
    pub filter: Expr,
    pub sort: TaskSort,
    pub limit: Option<u32>,
}

impl TaskQuery {
    pub fn new(filter: Expr) -> TaskQuery {
        TaskQuery {
            filter,
            sort: TaskSort::Smart,
            limit: None,
        }
    }
}

/// A full-text search hit with the matched text highlighted by `[` `]`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub task: Task,
    pub snippet: String,
}

impl Db<'_> {
    pub fn query_tasks(&self, query: &TaskQuery, ctx: &QueryCtx) -> Result<Vec<Task>> {
        let mut params = Vec::new();
        let where_sql = compile(&query.filter, ctx, &mut params);
        let limit = query
            .limit
            .map(|n| format!(" LIMIT {n}"))
            .unwrap_or_default();
        let sql = format!(
            "{TASK_SELECT} WHERE t.deleted_at IS NULL AND ({where_sql}) ORDER BY {}{limit}",
            query.sort.order_by()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), task_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count_tasks(&self, filter: &Expr, ctx: &QueryCtx) -> Result<u32> {
        let mut params = Vec::new();
        let where_sql = compile(filter, ctx, &mut params);
        let sql =
            format!("SELECT count(*) FROM tasks t WHERE t.deleted_at IS NULL AND ({where_sql})");
        Ok(self
            .conn
            .query_row(&sql, params_from_iter(params), |r| r.get(0))?)
    }

    /// Ranked full-text search over titles and notes (prefix matching).
    pub fn search(&self, text: &str, include_closed: bool, limit: u32) -> Result<Vec<SearchHit>> {
        let Some(fts) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT rowid, snippet(tasks_fts, -1, '[', ']', '…', 10)
               FROM tasks_fts WHERE tasks_fts MATCH ?1 ORDER BY bm25(tasks_fts) LIMIT ?2",
        )?;
        let ids: Vec<(TaskId, String)> = stmt
            .query_map(rusqlite::params![fts, limit * 4], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut hits = Vec::new();
        for (id, snippet) in ids {
            if let Some(task) = self.try_task(id)? {
                if include_closed || task.is_open() {
                    hits.push(SearchHit { task, snippet });
                }
            }
            if hits.len() >= limit as usize {
                break;
            }
        }
        Ok(hits)
    }
}

/// Each word becomes a quoted prefix term: `inv rep` → `"inv"* "rep"*`.
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn compile(expr: &Expr, ctx: &QueryCtx, params: &mut Vec<Value>) -> String {
    match expr {
        Expr::All => "1".into(),
        Expr::Not(inner) => format!("NOT ({})", compile(inner, ctx, params)),
        Expr::And(items) => join(items, " AND ", ctx, params),
        Expr::Or(items) => join(items, " OR ", ctx, params),
        Expr::Pred(pred) => compile_pred(pred, ctx, params),
    }
}

fn join(items: &[Expr], sep: &str, ctx: &QueryCtx, params: &mut Vec<Value>) -> String {
    if items.is_empty() {
        return "1".into();
    }
    items
        .iter()
        .map(|e| format!("({})", compile(e, ctx, params)))
        .collect::<Vec<_>>()
        .join(sep)
}

fn bind(params: &mut Vec<Value>, value: impl Into<Value>) -> String {
    params.push(value.into());
    format!("?{}", params.len())
}

fn compile_pred(pred: &Pred, ctx: &QueryCtx, params: &mut Vec<Value>) -> String {
    match pred {
        Pred::Project(path) => match path.split_once('/') {
            Some((parent, child)) => {
                let parent = bind(params, parent.to_owned());
                let child = bind(params, child.to_owned());
                format!(
                    "t.project_id IN (SELECT p.id FROM projects p JOIN projects pp ON pp.id = p.parent_id
                       WHERE pp.name = {parent} COLLATE NOCASE AND p.name = {child} COLLATE NOCASE)"
                )
            }
            None => {
                let name = bind(params, path.clone());
                format!(
                    "t.project_id IN (SELECT p.id FROM projects p LEFT JOIN projects pp ON pp.id = p.parent_id
                       WHERE p.name = {name} COLLATE NOCASE OR pp.name = {name} COLLATE NOCASE)"
                )
            }
        },
        Pred::NoProject => "t.project_id IS NULL".into(),
        Pred::Tag(tag) => {
            let tag = bind(params, tag.clone());
            format!(
                "EXISTS (SELECT 1 FROM task_tags tt JOIN tags g ON g.id = tt.tag_id
                   WHERE tt.task_id = t.id AND g.name = {tag} COLLATE NOCASE)"
            )
        }
        Pred::NoTags => "NOT EXISTS (SELECT 1 FROM task_tags tt WHERE tt.task_id = t.id)".into(),
        Pred::Status(status) => format!("t.status = {}", bind(params, status.as_str().to_owned())),
        Pred::AnyStatus => "1".into(),
        Pred::Priority(cmp, level) => {
            format!(
                "t.priority {} {}",
                cmp.sql(),
                bind(params, i64::from(*level))
            )
        }
        Pred::Due(cmp, date) => format!(
            "t.due_date IS NOT NULL AND t.due_date {} {}",
            cmp.sql(),
            bind(params, date.to_string())
        ),
        Pred::DueNone => "t.due_date IS NULL".into(),
        Pred::DueAny => "t.due_date IS NOT NULL".into(),
        Pred::Sched(cmp, date) => format!(
            "t.scheduled_date IS NOT NULL AND t.scheduled_date {} {}",
            cmp.sql(),
            bind(params, date.to_string())
        ),
        Pred::SchedNone => "t.scheduled_date IS NULL".into(),
        Pred::Created(cmp, date) => instant_cmp("t.created_at", *cmp, *date, ctx, params),
        Pred::Modified(cmp, date) => instant_cmp("t.updated_at", *cmp, *date, ctx, params),
        Pred::Done(cmp, date) => format!(
            "t.status <> 'open' AND {}",
            instant_cmp("t.completed_at", *cmp, *date, ctx, params)
        ),
        Pred::Text(text) => match fts_query(text) {
            Some(q) => format!(
                "t.id IN (SELECT rowid FROM tasks_fts WHERE tasks_fts MATCH {})",
                bind(params, q)
            ),
            None => "1".into(),
        },
        Pred::HasNotes => "t.notes <> ''".into(),
        Pred::IsRecurring => "t.recur_rule IS NOT NULL".into(),
        Pred::IsBlocked => "EXISTS (SELECT 1 FROM task_deps d JOIN tasks b ON b.id = d.blocker_id
               WHERE d.blocked_id = t.id AND b.status = 'open' AND b.deleted_at IS NULL)"
            .into(),
        Pred::IsSubtask => "t.parent_id IS NOT NULL".into(),
        Pred::HasSubtasks => {
            "EXISTS (SELECT 1 FROM tasks c WHERE c.parent_id = t.id AND c.deleted_at IS NULL)"
                .into()
        }
        Pred::IsOverdue => format!(
            "t.status = 'open' AND t.due_date IS NOT NULL AND t.due_date < {}",
            bind(params, ctx.today.to_string())
        ),
        Pred::Id(id) => format!("t.id = {}", bind(params, *id)),
        Pred::Wait(cmp, date) => format!(
            "t.wait_until IS NOT NULL AND t.wait_until {} {}",
            cmp.sql(),
            bind(params, date.to_string())
        ),
        Pred::WaitNone => "t.wait_until IS NULL AND NOT t.someday".into(),
        Pred::IsWaiting => format!(
            "t.wait_until IS NOT NULL AND t.wait_until > {}",
            bind(params, ctx.today.to_string())
        ),
        Pred::IsSomeday => "t.someday".into(),
        Pred::WaitAny => "1".into(),
    }
}

/// Compares a UTC-ms column with a local calendar day.
fn instant_cmp(
    column: &str,
    cmp: Cmp,
    date: Date,
    ctx: &QueryCtx,
    params: &mut Vec<Value>,
) -> String {
    let start = start_of_day_ms(date, &ctx.tz);
    let next = start_of_day_ms(date.tomorrow().unwrap_or(date), &ctx.tz);
    match cmp {
        Cmp::Eq => format!(
            "{column} >= {} AND {column} < {}",
            bind(params, start),
            bind(params, next)
        ),
        Cmp::Lt => format!("{column} < {}", bind(params, start)),
        Cmp::Le => format!("{column} < {}", bind(params, next)),
        Cmp::Gt => format!("{column} >= {}", bind(params, next)),
        Cmp::Ge => format!("{column} >= {}", bind(params, start)),
    }
}
