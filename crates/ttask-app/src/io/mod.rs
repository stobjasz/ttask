//! Import and export. Every format converts to or from [`ImportBatch`];
//! JSON is the lossless one (history and uids included). An import is one
//! undoable batch, matched on `uid` so re-importing updates instead of
//! duplicating.

mod formats;
mod github;
mod ics;
mod taskwarrior;

use std::collections::HashMap;

use jiff::civil::{Date, Time};
use serde::{Deserialize, Serialize};
use ttask_core::recur::RecurMode;
use ttask_core::{NewTask, Priority, ProjectId, Status, TaskId, jiff, new_uid};
use ttask_store::{
    EventKind, EventRecord, ProjectRecord, SnapshotChecklistItem, SnapshotReminder, TaskSnapshot,
};

pub use formats::{
    csv_export, csv_import, markdown_export, ticktick_import, todoist_import, todotxt_export,
    todotxt_import,
};
pub use github::github_import;
pub use ics::ics_export;
pub use taskwarrior::taskwarrior_import;

use crate::{App, AppError, Result};

pub const FORMAT_NAME: &str = "ttask";
pub const FORMAT_VERSION: u32 = 1;

/// The lossless JSON document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportDoc {
    pub format: String,
    pub version: u32,
    pub exported_at: i64,
    pub projects: Vec<ProjectRecord>,
    pub tags: Vec<ExportTag>,
    pub tasks: Vec<ExportTask>,
    #[serde(default)]
    pub saved_filters: Vec<ExportFilter>,
    #[serde(default)]
    pub history: Vec<ExportEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTag {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFilter {
    pub name: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEvent {
    pub task_uid: String,
    pub at: i64,
    pub kind: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTask {
    pub uid: String,
    pub project_uid: Option<String>,
    pub parent_uid: Option<String>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub status: Status,
    #[serde(default)]
    pub priority: Priority,
    pub due_date: Option<Date>,
    pub due_time: Option<Time>,
    pub scheduled_date: Option<Date>,
    pub estimate_min: Option<u32>,
    pub recur_rule: Option<String>,
    pub recur_mode: Option<RecurMode>,
    pub recur_series: Option<String>,
    #[serde(default)]
    pub wait_until: Option<Date>,
    #[serde(default)]
    pub someday: bool,
    #[serde(default)]
    pub sort_order: f64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub checklist: Vec<ExportChecklistItem>,
    #[serde(default)]
    pub reminders: Vec<ExportReminder>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub time_entries: Vec<ExportTimeEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportChecklistItem {
    pub text: String,
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportReminder {
    pub fire_at: i64,
    pub fired_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTimeEntry {
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// Format-neutral input to [`App::import`].
#[derive(Debug, Clone, Default)]
pub struct ImportBatch {
    /// Projects with stable uids (JSON); others are created from task paths.
    pub projects: Vec<ProjectRecord>,
    pub tags: Vec<ExportTag>,
    pub tasks: Vec<ImportTask>,
    pub saved_filters: Vec<ExportFilter>,
    pub history: Vec<ExportEvent>,
    /// Problems that did not stop the import.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportTask {
    /// Matched against existing tasks; a new uid is minted when absent.
    pub uid: Option<String>,
    /// Reference other rows use in `parent` / `blocked_by` (defaults to uid).
    pub key: Option<String>,
    pub project_uid: Option<String>,
    /// `name` or `parent/name`, used when `project_uid` is absent.
    pub project_path: Option<String>,
    pub parent: Option<String>,
    pub title: String,
    pub notes: String,
    pub status: Option<Status>,
    pub priority: Priority,
    pub due_date: Option<Date>,
    pub due_time: Option<Time>,
    pub scheduled_date: Option<Date>,
    pub estimate_min: Option<u32>,
    pub recur_rule: Option<String>,
    pub recur_mode: Option<RecurMode>,
    pub recur_series: Option<String>,
    pub wait_until: Option<Date>,
    pub someday: bool,
    pub sort_order: Option<f64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub tags: Vec<String>,
    pub checklist: Vec<(String, bool)>,
    pub reminders: Vec<(i64, Option<i64>)>,
    pub blocked_by: Vec<String>,
    pub time_entries: Vec<(i64, Option<i64>)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub tasks_created: usize,
    pub tasks_updated: usize,
    pub projects_created: usize,
    pub filters: usize,
    pub warnings: Vec<String>,
    pub dry_run: bool,
}

impl From<ExportDoc> for ImportBatch {
    fn from(doc: ExportDoc) -> Self {
        let tasks = doc
            .tasks
            .into_iter()
            .map(|t| ImportTask {
                key: Some(t.uid.clone()),
                uid: Some(t.uid),
                project_uid: t.project_uid,
                project_path: None,
                parent: t.parent_uid,
                title: t.title,
                notes: t.notes,
                status: Some(t.status),
                priority: t.priority,
                due_date: t.due_date,
                due_time: t.due_time,
                scheduled_date: t.scheduled_date,
                estimate_min: t.estimate_min,
                recur_rule: t.recur_rule,
                recur_mode: t.recur_mode,
                recur_series: t.recur_series,
                wait_until: t.wait_until,
                someday: t.someday,
                sort_order: Some(t.sort_order),
                created_at: Some(t.created_at),
                updated_at: Some(t.updated_at),
                completed_at: t.completed_at,
                tags: t.tags,
                checklist: t
                    .checklist
                    .into_iter()
                    .map(|c| (c.text, c.checked))
                    .collect(),
                reminders: t
                    .reminders
                    .into_iter()
                    .map(|r| (r.fire_at, r.fired_at))
                    .collect(),
                blocked_by: t.blocked_by,
                time_entries: t
                    .time_entries
                    .into_iter()
                    .map(|e| (e.started_at, e.ended_at))
                    .collect(),
            })
            .collect();
        ImportBatch {
            projects: doc.projects,
            tags: doc.tags,
            tasks,
            saved_filters: doc.saved_filters,
            history: doc.history,
            warnings: Vec::new(),
        }
    }
}

/// Parses an exported JSON document, checking its header.
pub fn parse_json(text: &str) -> Result<ExportDoc> {
    let doc: ExportDoc = serde_json::from_str(text)
        .map_err(|e| AppError::Invalid(format!("not a ttask JSON export: {e}")))?;
    if doc.format != FORMAT_NAME {
        return Err(AppError::Invalid(format!(
            "unknown format `{}`",
            doc.format
        )));
    }
    if doc.version > FORMAT_VERSION {
        return Err(AppError::Invalid(format!(
            "export version {} is newer than this ttask supports ({FORMAT_VERSION})",
            doc.version
        )));
    }
    Ok(doc)
}

impl App {
    /// Every live task, project, tag, saved filter and the history.
    pub fn export(&self) -> Result<ExportDoc> {
        let db = self.store().db();
        let projects = db.project_records()?;
        let project_uid: HashMap<ProjectId, String> = self
            .projects(true)?
            .into_iter()
            .map(|p| (p.id, p.uid))
            .collect();
        let mut uid_of: HashMap<TaskId, String> = HashMap::new();
        let mut tasks = Vec::new();
        for id in db.all_task_ids()? {
            let task = db.task(id)?;
            uid_of.insert(id, task.uid.clone());
            tasks.push(task);
        }
        let mut out = Vec::with_capacity(tasks.len());
        for task in tasks {
            let blocked_by = db
                .blockers(task.id)?
                .into_iter()
                .filter_map(|b| uid_of.get(&b).cloned())
                .collect();
            out.push(ExportTask {
                project_uid: task.project_id.and_then(|p| project_uid.get(&p).cloned()),
                parent_uid: task.parent_id.and_then(|p| uid_of.get(&p).cloned()),
                checklist: db
                    .checklist(task.id)?
                    .into_iter()
                    .map(|c| ExportChecklistItem {
                        text: c.text,
                        checked: c.checked,
                    })
                    .collect(),
                reminders: db
                    .reminders_for(task.id)?
                    .into_iter()
                    .map(|r| ExportReminder {
                        fire_at: r.fire_at,
                        fired_at: r.fired_at,
                    })
                    .collect(),
                time_entries: db
                    .time_entries(task.id)?
                    .into_iter()
                    .map(|e| ExportTimeEntry {
                        started_at: e.started_at,
                        ended_at: e.ended_at,
                    })
                    .collect(),
                blocked_by,
                uid: task.uid,
                title: task.title,
                notes: task.notes,
                status: task.status,
                priority: task.priority,
                due_date: task.due_date,
                due_time: task.due_time,
                scheduled_date: task.scheduled_date,
                estimate_min: task.estimate_min,
                recur_rule: task.recur_rule,
                recur_mode: task.recur_mode,
                recur_series: task.recur_series,
                wait_until: task.wait_until,
                someday: task.someday,
                sort_order: task.sort_order,
                created_at: task.created_at,
                updated_at: task.updated_at,
                completed_at: task.completed_at,
                tags: task.tags,
            });
        }
        let history = db
            .event_records()?
            .into_iter()
            .filter_map(|e| {
                let uid = uid_of.get(&e.task_id?)?.clone();
                Some(ExportEvent {
                    task_uid: uid,
                    at: e.at,
                    kind: e.kind,
                    before: e.before,
                    after: e.after,
                })
            })
            .collect();
        Ok(ExportDoc {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            exported_at: self.now_ms(),
            projects,
            tags: self
                .tags()?
                .into_iter()
                .map(|t| ExportTag {
                    name: t.name,
                    color: t.color,
                })
                .collect(),
            tasks: out,
            saved_filters: self
                .saved_filters()?
                .into_iter()
                .map(|f| ExportFilter {
                    name: f.name,
                    query: f.query,
                })
                .collect(),
            history,
        })
    }

    /// Imports a batch as one undoable action. With `dry_run` nothing is
    /// written and the report says what would have changed. `default_project`
    /// receives tasks that have no project of their own.
    pub fn import(
        &mut self,
        batch: ImportBatch,
        default_project: Option<&str>,
        dry_run: bool,
    ) -> Result<ImportReport> {
        let now = self.now_ms();
        let mut report = ImportReport {
            warnings: batch.warnings.clone(),
            dry_run,
            ..ImportReport::default()
        };
        let result = self.batch(|tx| {
            let db = tx.db;
            // Projects come from uids (JSON) and from paths (other formats).
            let projects_before = db.projects(true)?.len();
            // Projects with uids, parents first.
            let mut project_ids: HashMap<String, ProjectId> = HashMap::new();
            let mut pending: Vec<&ProjectRecord> = batch.projects.iter().collect();
            while !pending.is_empty() {
                let before = pending.len();
                let mut rest = Vec::new();
                for p in pending {
                    let parent = match &p.parent_uid {
                        None => None,
                        Some(uid) => match project_ids.get(uid) {
                            Some(id) => Some(*id),
                            None => {
                                rest.push(p);
                                continue;
                            }
                        },
                    };
                    let (id, _created) = db.upsert_project(p, parent)?;
                    project_ids.insert(p.uid.clone(), id);
                }
                if rest.len() == before {
                    for p in &rest {
                        report.warnings.push(format!(
                            "project `{}` has a missing parent; skipped",
                            p.name
                        ));
                    }
                    break;
                }
                pending = rest;
            }
            for tag in &batch.tags {
                match db.ensure_tag(&tag.name) {
                    Ok(_) if tag.color.is_some() => {
                        db.set_tag_color(&tag.name, tag.color.as_deref())?
                    }
                    Ok(_) => {}
                    Err(e) => report.warnings.push(format!("tag `{}`: {e}", tag.name)),
                }
            }
            let default_project_id = default_project
                .map(|path| {
                    let segments: Vec<String> = path.split('/').map(str::to_owned).collect();
                    db.ensure_project_path(&segments, now)
                })
                .transpose()?;

            // Pass 1: create or update each task (no links yet).
            let mut ids_by_key: HashMap<String, TaskId> = HashMap::new();
            let mut imported: Vec<(TaskId, &ImportTask, bool)> = Vec::new();
            for item in &batch.tasks {
                if item.title.trim().is_empty() {
                    report.warnings.push("skipped a row without a title".into());
                    continue;
                }
                let project_id = match (&item.project_uid, &item.project_path) {
                    (Some(uid), _) => match project_ids.get(uid) {
                        Some(id) => Some(*id),
                        None => db.project_id_by_uid(uid)?,
                    },
                    (None, Some(path)) if !path.trim().is_empty() => {
                        let segments: Vec<String> =
                            path.split('/').map(|s| s.trim().to_owned()).collect();
                        match db.ensure_project_path(&segments, now) {
                            Ok(id) => Some(id),
                            Err(e) => {
                                report.warnings.push(format!("`{}`: {e}", item.title));
                                default_project_id
                            }
                        }
                    }
                    _ => default_project_id,
                };
                let uid = item.uid.clone().unwrap_or_else(new_uid);
                let existing = db.task_id_by_uid(&uid)?;
                let id = match existing {
                    Some(id) => id,
                    None => {
                        let draft = NewTask {
                            title: item.title.clone(),
                            ..NewTask::default()
                        };
                        db.insert_task_with_uid(&draft, &uid, now)?.id
                    }
                };
                let mut snap = db.snapshot(id)?.expect("just written");
                let before = existing.map(|_| snap.clone());
                fill_snapshot(&mut snap, item, project_id, now);
                // Tags are validated here so one bad tag does not abort the import.
                snap.tags.retain(|t| {
                    let ok = ttask_core::quickadd::is_tag_name(t);
                    if !ok {
                        report
                            .warnings
                            .push(format!("`{}`: dropped invalid tag `{t}`", item.title));
                    }
                    ok
                });
                db.restore(&snap)?;
                db.replace_time_entries(id, &item.time_entries)?;
                let after = db.snapshot(id)?;
                let kind = if existing.is_some() {
                    report.tasks_updated += 1;
                    EventKind::Updated
                } else {
                    report.tasks_created += 1;
                    EventKind::Imported
                };
                db.record_event(tx.id, id, kind, before.as_ref(), after.as_ref(), now)?;
                ids_by_key.insert(item.key.clone().unwrap_or(uid), id);
                imported.push((id, item, existing.is_none()));
            }

            // Pass 2: parents and dependencies, which may point forward.
            let resolve = |key: &str| -> ttask_store::Result<Option<TaskId>> {
                match ids_by_key.get(key) {
                    Some(id) => Ok(Some(*id)),
                    None => db.task_id_by_uid(key),
                }
            };
            for (id, item, _) in &imported {
                let parent = match &item.parent {
                    Some(key) => resolve(key)?,
                    None => None,
                };
                let blockers: Vec<TaskId> = item
                    .blocked_by
                    .iter()
                    .filter_map(|k| resolve(k).transpose())
                    .collect::<ttask_store::Result<_>>()?;
                if parent.is_none() && blockers.is_empty() {
                    continue;
                }
                let mut snap = db.snapshot(*id)?.expect("imported");
                if let Some(parent) = parent {
                    let parent_is_top_level =
                        db.snapshot(parent)?.is_some_and(|p| p.parent_id.is_none());
                    if parent != *id && parent_is_top_level {
                        snap.parent_id = Some(parent);
                    } else {
                        report.warnings.push(format!(
                            "`{}`: subtasks nest one level; kept top-level",
                            item.title
                        ));
                    }
                }
                snap.blockers = blockers;
                db.restore(&snap)?;
            }

            // History for tasks that are new here.
            for event in &batch.history {
                if let Some((id, _, true)) = imported
                    .iter()
                    .find(|(_, item, _)| item.uid.as_deref() == Some(event.task_uid.as_str()))
                {
                    db.insert_history(
                        *id,
                        &EventRecord {
                            task_id: Some(*id),
                            at: event.at,
                            kind: event.kind.clone(),
                            before: event.before.clone(),
                            after: event.after.clone(),
                        },
                    )?;
                }
            }
            for filter in &batch.saved_filters {
                db.save_filter(&filter.name, &filter.query)?;
                report.filters += 1;
            }
            report.projects_created = db.projects(true)?.len().saturating_sub(projects_before);
            if dry_run {
                // Roll everything back but keep the report.
                return Err(AppError::DryRun);
            }
            Ok(())
        });
        match result {
            Ok(()) | Err(AppError::DryRun) => Ok(report),
            Err(e) => Err(e),
        }
    }
}

fn fill_snapshot(
    snap: &mut TaskSnapshot,
    item: &ImportTask,
    project_id: Option<ProjectId>,
    now: i64,
) {
    let status = item.status.unwrap_or(Status::Open);
    snap.project_id = project_id;
    snap.parent_id = None;
    snap.title = item.title.trim().to_owned();
    snap.notes = item.notes.clone();
    snap.status = status.as_str().to_owned();
    snap.priority = item.priority.value();
    snap.due_date = item.due_date.map(|d| d.to_string());
    snap.due_time = item.due_time.map(ttask_core::date::format_time);
    snap.scheduled_date = item.scheduled_date.map(|d| d.to_string());
    snap.estimate_min = item.estimate_min;
    snap.recur_rule = item.recur_rule.clone();
    snap.recur_mode = item.recur_mode.map(|m| m.as_str().to_owned());
    snap.recur_series = item.recur_series.clone();
    snap.wait_until = item.wait_until.map(|d| d.to_string());
    snap.someday = item.someday;
    if let Some(order) = item.sort_order {
        snap.sort_order = order;
    }
    if let Some(created) = item.created_at {
        snap.created_at = created;
    }
    snap.updated_at = item.updated_at.unwrap_or(now);
    snap.completed_at = match status {
        Status::Open => None,
        _ => item.completed_at.or(Some(now)),
    };
    snap.deleted_at = None;
    snap.tags = item.tags.clone();
    snap.checklist = item
        .checklist
        .iter()
        .enumerate()
        .map(|(i, (text, checked))| SnapshotChecklistItem {
            text: text.clone(),
            checked: *checked,
            sort_order: i as f64 + 1.0,
        })
        .collect();
    snap.reminders = item
        .reminders
        .iter()
        .map(|(fire_at, fired_at)| SnapshotReminder {
            fire_at: *fire_at,
            fired_at: *fired_at,
        })
        .collect();
    snap.blockers = Vec::new();
}
