//! The TOML form `ttask edit <id>` opens in `$EDITOR`.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use ttask_core::Task;
use ttask_core::form::TaskForm;
use ttask_core::jiff::civil::Date;
use ttask_core::quickadd::Mods;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Form {
    title: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    due: String,
    #[serde(default)]
    scheduled: String,
    #[serde(default)]
    wait: String,
    #[serde(default)]
    estimate: String,
    #[serde(default)]
    repeat: String,
    #[serde(default)]
    notes: String,
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

pub fn render(task: &Task) -> String {
    let f = TaskForm::from_task(task);
    let tags: Vec<String> = f.tags.iter().map(|t| toml_string(t)).collect();
    let notes = f.notes.replace("'''", "' ' '");
    format!(
        "# Edit task {id}. Lines starting with # are ignored. Save and close to apply;\n\
         # leave the file unchanged to cancel. Empty strings clear a field.\n\
         title = {title}\n\
         project = {project}     # name or parent/name\n\
         tags = [{tags}]\n\
         priority = {priority}   # low, medium, high, urgent\n\
         due = {due}         # 2026-11-30, 2026-11-30@17:00, fri, +3d\n\
         scheduled = {scheduled}\n\
         wait = {wait}         # hide until a date, or someday\n\
         estimate = {estimate}    # 30m, 2h\n\
         repeat = {repeat}      # week, weekday, month@15, FREQ=...; prefix ! for after completion\n\
         notes = '''\n{notes}\n'''\n",
        id = task.id,
        title = toml_string(&f.title),
        project = toml_string(&f.project),
        tags = tags.join(", "),
        priority = toml_string(&f.priority),
        due = toml_string(&f.due),
        wait = toml_string(&f.wait),
        scheduled = toml_string(&f.scheduled),
        estimate = toml_string(&f.estimate),
        repeat = toml_string(&f.repeat),
    )
}

/// Parses the saved form into the modifications it makes to `task`.
pub fn parse(text: &str, task: &Task, today: Date) -> anyhow::Result<Mods> {
    let form: Form = toml::from_str(text).context("reading the edit form")?;
    let edited = TaskForm {
        title: form.title,
        project: form.project,
        tags: form.tags,
        priority: form.priority,
        due: form.due,
        scheduled: form.scheduled,
        wait: form.wait,
        estimate: form.estimate,
        repeat: form.repeat,
        notes: form.notes.trim_matches('\n').to_owned(),
    };
    edited
        .changes(&TaskForm::from_task(task), today)
        .map_err(|e| anyhow!(e))
}

#[cfg(test)]
mod tests {
    use ttask_core::jiff::civil::date;
    use ttask_core::quickadd::Change;
    use ttask_core::recur::RecurMode;
    use ttask_core::{Priority, Progress, Status};

    use super::*;

    fn task() -> Task {
        Task {
            id: 1,
            uid: "u1".into(),
            parent_id: None,
            project_id: None,
            project: None,
            title: "Old".into(),
            notes: "old notes".into(),
            status: Status::Open,
            priority: Priority::NONE,
            due_date: None,
            due_time: None,
            scheduled_date: None,
            estimate_min: None,
            recur_rule: None,
            recur_mode: None,
            recur_series: None,
            wait_until: None,
            someday: false,
            tags: vec!["x".into()],
            sort_order: 0.0,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            subtasks: Progress::default(),
            checklist: Progress::default(),
            blocked: false,
            blocking: false,
            urgency: 0.0,
        }
    }

    #[test]
    fn form_round_trips_and_clears() {
        let task = task();
        let unchanged = render(&task);
        assert!(
            parse(&unchanged, &task, date(2026, 10, 15))
                .unwrap()
                .is_empty()
        );

        let form = "title = \"New title\"\nproject = \"work/acme\"\ntags = [\"a\", \"+b\"]\n\
                    priority = \"high\"\ndue = \"2026-11-30@17:00\"\nscheduled = \"\"\n\
                    estimate = \"2h\"\nrepeat = \"!3d\"\nnotes = '''\nline 1\nline 2\n'''\n";
        let mods = parse(form, &task, date(2026, 10, 15)).unwrap();
        assert_eq!(mods.title.as_deref(), Some("New title"));
        assert_eq!(mods.add_tags, ["a", "b"]);
        assert_eq!(mods.remove_tags, ["x"]);
        assert_eq!(mods.priority, Priority::new(2));
        assert_eq!(mods.sched, None);
        assert_eq!(mods.estimate, Some(Change::Set(120)));
        let (_, mode) = mods.recur.unwrap().into_option().unwrap();
        assert_eq!(mode, RecurMode::Completion);
        assert_eq!(mods.notes.as_deref(), Some("line 1\nline 2"));
        assert!(parse("title = 1", &task, date(2026, 10, 15)).is_err());
    }
}
