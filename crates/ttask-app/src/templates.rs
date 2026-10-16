//! Templates: a task with its subtasks and checklists, or a whole
//! project's open tasks, saved under a name and created again on demand.
//! Dates are kept as days after the day the template was saved, and land
//! the same distance after the day it is used.

use jiff::ToSpan;
use jiff::civil::{Date, Time};
use serde::{Deserialize, Serialize};
use ttask_core::form::parse_repeat;
use ttask_core::quickadd::parse_mods;
use ttask_core::recur::RecurMode;
use ttask_core::{NewTask, Priority, ProjectId, Task, TaskId, jiff};
use ttask_store::{EventKind, StoreError, TaskFields};

use crate::{App, AppError, Batch, Result, apply_mods, set_reminders};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    /// One task (with subtasks).
    Task,
    /// Every open task of a project.
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub kind: TemplateKind,
    /// `name` or `parent/name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub tasks: Vec<TemplateTask>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateTask {
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// `low` … `urgent`.
    pub priority: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_min: Option<u32>,
    /// Days after the day of use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_in_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_time: Option<Time>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_in_days: Option<i64>,
    /// As in the edit form: an RRULE, `!` first for after completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checklist: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subtasks: Vec<TemplateTask>,
}

impl Template {
    pub fn task_count(&self) -> usize {
        self.tasks.iter().map(|t| 1 + t.subtasks.len()).sum()
    }
}

/// How to create tasks from a template.
#[derive(Debug, Clone, Default)]
pub struct UseTemplate {
    /// Dates count from this day (default: today).
    pub start: Option<Date>,
    /// Quick-add text applied to a task template's task (`Ship v2 due:fri`).
    pub text: Option<String>,
    /// Project to create the tasks in, instead of the template's.
    pub project: Option<String>,
}

impl App {
    pub fn templates(&self) -> Result<Vec<Template>> {
        self.store()
            .db()
            .templates()?
            .into_iter()
            .map(|row| parse_body(&row.body))
            .collect()
    }

    pub fn template(&self, name: &str) -> Result<Template> {
        parse_body(&self.store().db().template(name)?.body)
    }

    /// Saves task `id` (with its subtasks and checklists) as a template.
    pub fn save_task_template(&mut self, name: &str, id: TaskId) -> Result<Template> {
        let today = self.today();
        let detail = self.detail(id)?;
        let mut root = self.template_task(&detail.task, today)?;
        for sub in &detail.subtasks {
            root.subtasks.push(self.template_task(sub, today)?);
        }
        let template = Template {
            name: name.trim().to_owned(),
            kind: TemplateKind::Task,
            project: detail.task.project.clone(),
            tasks: vec![root],
        };
        self.store_template(&template)?;
        Ok(template)
    }

    /// Saves the open tasks of project `path` as a template.
    pub fn save_project_template(&mut self, name: &str, path: &str) -> Result<Template> {
        let today = self.today();
        let project = self.find_project(path)?;
        let query = format!("project:\"{}\" and not is:subtask", project.path);
        let roots = self.list_with(&query, "wait:any", ttask_store::TaskSort::Manual, false)?;
        let mut tasks = Vec::new();
        for root in &roots {
            let mut t = self.template_task(root, today)?;
            for sub in self
                .detail(root.id)?
                .subtasks
                .iter()
                .filter(|s| s.is_open())
            {
                t.subtasks.push(self.template_task(sub, today)?);
            }
            tasks.push(t);
        }
        if tasks.is_empty() {
            return Err(AppError::Invalid(format!(
                "project {} has no open tasks to save",
                project.path
            )));
        }
        let template = Template {
            name: name.trim().to_owned(),
            kind: TemplateKind::Project,
            project: Some(project.path),
            tasks,
        };
        self.store_template(&template)?;
        Ok(template)
    }

    pub fn delete_template(&mut self, name: &str) -> Result<()> {
        Ok(self.store_mut().write(|db| db.delete_template(name))?)
    }

    /// Creates the template's tasks as one undo step; returns the top-level ones.
    pub fn use_template(&mut self, name: &str, opts: &UseTemplate) -> Result<Vec<Task>> {
        let template = self.template(name)?;
        let today = self.today();
        let start = opts.start.unwrap_or(today);
        let mods = match opts
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(text) if template.kind == TemplateKind::Task => Some(parse_mods(text, today)?),
            Some(_) => {
                return Err(AppError::Invalid(
                    "a project template takes --project, not task text".into(),
                ));
            }
            None => None,
        };
        let project = opts.project.clone().or(template.project.clone());
        let (tz, now) = (self.clock().time_zone(), self.now_ms());
        self.batch(|tx| {
            let project_id = match project.as_deref().map(str::trim) {
                Some(path) if !path.is_empty() => {
                    let segments: Vec<String> = path
                        .trim_start_matches('#')
                        .split('/')
                        .map(|s| s.trim().to_owned())
                        .collect();
                    Some(tx.db.ensure_project_path(&segments, now)?)
                }
                _ => None,
            };
            let mut roots = Vec::new();
            for t in &template.tasks {
                let root = create(tx, t, start, project_id, None, now)?;
                if let Some(mods) = &mods {
                    let mut fields = TaskFields::from(&tx.db.task(root)?);
                    apply_mods(tx.db, &mut fields, mods, today, now)?;
                    let task = tx.db.save_task(root, &fields, now)?;
                    set_reminders(tx.db, &task, mods, today, &tz)?;
                }
                for sub in &t.subtasks {
                    let id = create(tx, sub, start, project_id, Some(root), now)?;
                    tx.record_new(id, EventKind::Created)?;
                }
                tx.record_new(root, EventKind::Created)?;
                roots.push(tx.db.task(root)?);
            }
            Ok(roots)
        })
    }

    fn template_task(&self, task: &Task, today: Date) -> Result<TemplateTask> {
        let checklist = self
            .detail(task.id)?
            .checklist
            .into_iter()
            .map(|c| c.text)
            .collect();
        Ok(TemplateTask {
            title: task.title.clone(),
            notes: task.notes.clone(),
            priority: task.priority.name().to_owned(),
            tags: task.tags.clone(),
            estimate_min: task.estimate_min,
            due_in_days: task
                .due_date
                .map(|d| ttask_core::date::days_between(today, d)),
            due_time: task.due_time,
            scheduled_in_days: task
                .scheduled_date
                .map(|d| ttask_core::date::days_between(today, d)),
            repeat: task.recur_rule.as_ref().map(|r| match task.recur_mode {
                Some(RecurMode::Completion) => format!("!{r}"),
                _ => r.clone(),
            }),
            checklist,
            subtasks: Vec::new(),
        })
    }

    fn store_template(&mut self, template: &Template) -> Result<()> {
        let body = serde_json::to_string(template)
            .map_err(|e| AppError::Invalid(format!("template: {e}")))?;
        let now = self.now_ms();
        Ok(self
            .store_mut()
            .write(|db| db.save_template(&template.name, &body, now))?)
    }
}

fn parse_body(body: &str) -> Result<Template> {
    serde_json::from_str(body)
        .map_err(|e| StoreError::Invalid(format!("corrupt template: {e}")).into())
}

/// Inserts one template task with its checklist (not yet journalled).
fn create(
    tx: &Batch<'_, '_>,
    t: &TemplateTask,
    start: Date,
    project_id: Option<ProjectId>,
    parent: Option<TaskId>,
    now: i64,
) -> Result<TaskId> {
    let recur = match t.repeat.as_deref() {
        Some(r) => Some(parse_repeat(r).map_err(AppError::Invalid)?),
        None => None,
    };
    let draft = NewTask {
        title: t.title.clone(),
        notes: t.notes.clone(),
        priority: Priority::from_name(&t.priority).unwrap_or_default(),
        project_id,
        parent_id: parent,
        due_date: t.due_in_days.map(|d| start + d.days()),
        due_time: t.due_time.filter(|_| t.due_in_days.is_some()),
        scheduled_date: t.scheduled_in_days.map(|d| start + d.days()),
        estimate_min: t.estimate_min,
        recur_rule: recur.as_ref().map(|(rule, _)| rule.to_rrule()),
        recur_mode: recur.map(|(_, mode)| mode),
        tags: t.tags.clone(),
        ..NewTask::default()
    };
    let task = tx.db.insert_task(&draft, now)?;
    for item in &t.checklist {
        tx.db.add_checklist_item(task.id, item)?;
    }
    Ok(task.id)
}
