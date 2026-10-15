//! The TOML form `ttask edit <id>` opens in `$EDITOR`.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use ttask_core::date::{format_time, parse_date};
use ttask_core::duration::{format_minutes, parse_minutes};
use ttask_core::jiff::civil::Date;
use ttask_core::quickadd::{Change, Mods};
use ttask_core::recur::{RecurMode, RecurRule};
use ttask_core::{Priority, Task};

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
    let due = task
        .due_date
        .map(|d| match task.due_time {
            Some(t) => format!("{d}@{}", format_time(t)),
            None => d.to_string(),
        })
        .unwrap_or_default();
    let repeat = task
        .recur_rule
        .as_ref()
        .map(|r| match task.recur_mode {
            Some(RecurMode::Completion) => format!("!{r}"),
            _ => r.clone(),
        })
        .unwrap_or_default();
    let tags: Vec<String> = task.tags.iter().map(|t| toml_string(t)).collect();
    let notes = task.notes.replace("'''", "' ' '");
    format!(
        "# Edit task {id}. Lines starting with # are ignored. Save and close to apply;\n\
         # leave the file unchanged to cancel. Empty strings clear a field.\n\
         title = {title}\n\
         project = {project}     # name or parent/name\n\
         tags = [{tags}]\n\
         priority = {priority}   # low, medium, high, urgent\n\
         due = {due}         # 2026-11-30, 2026-11-30@17:00, fri, +3d\n\
         scheduled = {scheduled}\n\
         estimate = {estimate}    # 30m, 2h\n\
         repeat = {repeat}      # week, weekday, month@15, FREQ=...; prefix ! for after completion\n\
         notes = '''\n{notes}\n'''\n",
        id = task.id,
        title = toml_string(&task.title),
        project = toml_string(task.project.as_deref().unwrap_or("")),
        tags = tags.join(", "),
        priority = toml_string(task.priority.name()),
        due = toml_string(&due),
        scheduled = toml_string(
            &task
                .scheduled_date
                .map(|d| d.to_string())
                .unwrap_or_default()
        ),
        estimate = toml_string(&task.estimate_min.map(format_minutes).unwrap_or_default()),
        repeat = toml_string(&repeat),
    )
}

/// Parses the saved form into modifications and the new notes.
pub fn parse(text: &str, today: Date) -> anyhow::Result<(Mods, String)> {
    let form: Form = toml::from_str(text).context("reading the edit form")?;
    let mut mods = Mods {
        title: Some(form.title.trim().to_owned()),
        ..Mods::default()
    };
    let project = form.project.trim().trim_start_matches('#');
    mods.project = Some(if project.is_empty() {
        Change::Clear
    } else {
        Change::Set(project.split('/').map(|s| s.trim().to_owned()).collect())
    });
    mods.add_tags = form
        .tags
        .iter()
        .map(|t| t.trim().trim_start_matches('+').to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    mods.priority = Some(if form.priority.trim().is_empty() {
        Priority::NONE
    } else {
        Priority::from_name(&form.priority.trim().to_ascii_lowercase())
            .ok_or_else(|| anyhow!("priority is low, medium, high or urgent"))?
    });
    mods.due = Some(match form.due.trim() {
        "" => Change::Clear,
        due => Change::Set(parse_date(due, today)?),
    });
    mods.sched = Some(match form.scheduled.trim() {
        "" => Change::Clear,
        s => Change::Set(parse_date(s, today)?.date),
    });
    mods.estimate = Some(match form.estimate.trim() {
        "" => Change::Clear,
        e => Change::Set(parse_minutes(e).ok_or_else(|| anyhow!("estimate looks like 30m or 2h"))?),
    });
    mods.recur = Some(match form.repeat.trim() {
        "" => Change::Clear,
        r => {
            let (mode, rule) = match r.strip_prefix('!') {
                Some(rest) => (RecurMode::Completion, rest),
                None => (RecurMode::Due, r),
            };
            Change::Set((RecurRule::from_shorthand(rule)?, mode))
        }
    });
    Ok((mods, form.notes.trim_matches('\n').to_owned()))
}

#[cfg(test)]
mod tests {
    use ttask_core::jiff::civil::date;

    use super::*;

    #[test]
    fn form_round_trips_and_clears() {
        let form = "title = \"New title\"\nproject = \"work/acme\"\ntags = [\"a\", \"+b\"]\n\
                    priority = \"high\"\ndue = \"2026-11-30@17:00\"\nscheduled = \"\"\n\
                    estimate = \"2h\"\nrepeat = \"!3d\"\nnotes = '''\nline 1\nline 2\n'''\n";
        let (mods, notes) = parse(form, date(2026, 10, 15)).unwrap();
        assert_eq!(mods.title.as_deref(), Some("New title"));
        assert_eq!(mods.add_tags, ["a", "b"]);
        assert_eq!(mods.priority, Priority::new(2));
        assert_eq!(mods.sched, Some(Change::Clear));
        assert_eq!(mods.estimate, Some(Change::Set(120)));
        let (_, mode) = mods.recur.unwrap().into_option().unwrap();
        assert_eq!(mode, RecurMode::Completion);
        assert_eq!(notes, "line 1\nline 2");
        assert!(parse("title = 1", date(2026, 10, 15)).is_err());
    }
}
