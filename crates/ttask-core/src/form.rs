//! A task's editable fields as text: what the TUI editor shows and what
//! `ttask edit`'s TOML form holds. Turning a filled-in form back into
//! [`Mods`] only touches the fields that changed.

use jiff::civil::Date;

use crate::date::{format_time, parse_date};
use crate::duration::{format_minutes, parse_minutes};
use crate::quickadd::{Change, Mods, Wait, is_tag_name};
use crate::recur::{RecurMode, RecurRule};
use crate::{Priority, Task};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskForm {
    pub title: String,
    /// `name` or `parent/name`; empty is the Inbox.
    pub project: String,
    pub tags: Vec<String>,
    /// `low`, `medium`, `high`, `urgent`.
    pub priority: String,
    /// A date phrase, optionally with `@time` (`fri@17:00`).
    pub due: String,
    pub scheduled: String,
    /// A date, or `someday`.
    pub wait: String,
    /// `30m`, `2h`.
    pub estimate: String,
    /// `week`, `month@15`, `FREQ=…`; a leading `!` repeats after completion.
    pub repeat: String,
    pub notes: String,
}

impl TaskForm {
    pub fn from_task(task: &Task) -> TaskForm {
        TaskForm {
            title: task.title.clone(),
            project: task.project.clone().unwrap_or_default(),
            tags: task.tags.clone(),
            priority: task.priority.name().to_owned(),
            due: task
                .due_date
                .map(|d| match task.due_time {
                    Some(t) => format!("{d}@{}", format_time(t)),
                    None => d.to_string(),
                })
                .unwrap_or_default(),
            scheduled: task
                .scheduled_date
                .map(|d| d.to_string())
                .unwrap_or_default(),
            wait: if task.someday {
                "someday".to_owned()
            } else {
                task.wait_until.map(|d| d.to_string()).unwrap_or_default()
            },
            estimate: task.estimate_min.map(format_minutes).unwrap_or_default(),
            repeat: task
                .recur_rule
                .as_ref()
                .map(|r| match task.recur_mode {
                    Some(RecurMode::Completion) => format!("!{r}"),
                    _ => r.clone(),
                })
                .unwrap_or_default(),
            notes: task.notes.clone(),
        }
    }

    /// Modifications that turn `original` into this form. Unchanged fields
    /// are left alone, so a task's exact RRULE or due time survives an edit
    /// of its title. Errors name the field.
    pub fn changes(&self, original: &TaskForm, today: Date) -> Result<Mods, String> {
        let mut mods = Mods::default();
        let changed = |a: &str, b: &str| a.trim() != b.trim();
        if changed(&self.title, &original.title) {
            let title = self.title.trim();
            if title.is_empty() {
                return Err("title: a task needs a title".into());
            }
            mods.title = Some(title.to_owned());
        }
        if changed(&self.project, &original.project) {
            mods.project = Some(parse_project(&self.project));
        }
        let tags = normalise_tags(&self.tags)?;
        let before = normalise_tags(&original.tags).unwrap_or_default();
        mods.add_tags = tags
            .iter()
            .filter(|t| !before.iter().any(|b| b.eq_ignore_ascii_case(t)))
            .cloned()
            .collect();
        mods.remove_tags = before
            .iter()
            .filter(|b| !tags.iter().any(|t| t.eq_ignore_ascii_case(b)))
            .cloned()
            .collect();
        if changed(&self.priority, &original.priority) {
            mods.priority = Some(parse_priority(&self.priority)?);
        }
        if changed(&self.due, &original.due) {
            mods.due = Some(match self.due.trim() {
                "" => Change::Clear,
                due => Change::Set(parse_due(due, today).map_err(|e| format!("due: {e}"))?),
            });
        }
        if changed(&self.scheduled, &original.scheduled) {
            mods.sched = Some(match self.scheduled.trim() {
                "" => Change::Clear,
                s => Change::Set(
                    parse_date(s, today)
                        .map_err(|e| format!("scheduled: {e}"))?
                        .date,
                ),
            });
        }
        if changed(&self.wait, &original.wait) {
            mods.wait = Some(parse_wait(&self.wait, today).map_err(|e| format!("wait: {e}"))?);
        }
        if changed(&self.estimate, &original.estimate) {
            mods.estimate = Some(match self.estimate.trim() {
                "" => Change::Clear,
                e => Change::Set(parse_minutes(e).ok_or("estimate: looks like 30m, 2h or 1h30m")?),
            });
        }
        if changed(&self.repeat, &original.repeat) {
            mods.recur = Some(match self.repeat.trim() {
                "" => Change::Clear,
                r => Change::Set(parse_repeat(r).map_err(|e| format!("repeat: {e}"))?),
            });
        }
        if self.notes.trim_end() != original.notes.trim_end() {
            mods.notes = Some(self.notes.trim_end().to_owned());
        }
        Ok(mods)
    }
}

pub fn parse_project(text: &str) -> Change<Vec<String>> {
    let project = text.trim().trim_start_matches('#');
    if project.is_empty() || project.eq_ignore_ascii_case("inbox") {
        Change::Clear
    } else {
        Change::Set(project.split('/').map(|s| s.trim().to_owned()).collect())
    }
}

/// `+a b, c` → `[a, b, c]`.
pub fn split_tags(text: &str) -> Vec<String> {
    text.split([' ', ','])
        .map(|t| t.trim().trim_start_matches(['+', '#']).to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

fn normalise_tags(tags: &[String]) -> Result<Vec<String>, String> {
    let tags: Vec<String> = tags.iter().flat_map(|t| split_tags(t)).collect();
    match tags.iter().find(|t| !is_tag_name(t)) {
        Some(bad) => Err(format!("tags: `{bad}` is not a tag (start with a letter)")),
        None => Ok(tags),
    }
}

pub fn parse_priority(text: &str) -> Result<Priority, String> {
    let text = text.trim().trim_start_matches('!').to_ascii_lowercase();
    if text.is_empty() {
        return Ok(Priority::NONE);
    }
    Priority::from_name(&text)
        .or_else(|| text.parse::<u8>().ok().and_then(Priority::new))
        .ok_or_else(|| "priority: low, medium, high or urgent".into())
}

/// A due date with an optional time after `@` or a space (`tom 17:00`).
pub fn parse_due(text: &str, today: Date) -> Result<crate::date::DateSpec, String> {
    parse_date(&text.trim().replace(' ', "@"), today)
        .or_else(|_| parse_date(text.trim(), today))
        .map_err(|e| e.to_string())
}

pub fn parse_wait(text: &str, today: Date) -> Result<Change<Wait>, String> {
    match text.trim() {
        "" => Ok(Change::Clear),
        w if w.eq_ignore_ascii_case("none") => Ok(Change::Clear),
        w if w.eq_ignore_ascii_case("someday") => Ok(Change::Set(Wait::Someday)),
        w => parse_date(w, today)
            .map(|spec| Change::Set(Wait::Until(spec.date)))
            .map_err(|e| e.to_string()),
    }
}

pub fn parse_repeat(text: &str) -> Result<(RecurRule, RecurMode), String> {
    let (mode, rule) = match text.trim().strip_prefix('!') {
        Some(rest) => (RecurMode::Completion, rest),
        None => (RecurMode::Due, text.trim()),
    };
    RecurRule::from_shorthand(rule)
        .map(|rule| (rule, mode))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 15);

    fn form() -> TaskForm {
        TaskForm {
            title: "Old".into(),
            project: "work".into(),
            tags: vec!["a".into(), "b".into()],
            priority: "low".into(),
            repeat: "FREQ=WEEKLY;BYDAY=MO".into(),
            ..TaskForm::default()
        }
    }

    #[test]
    fn only_changed_fields_become_mods() {
        let original = form();
        assert!(original.changes(&original, TODAY).unwrap().is_empty());

        let mut edited = original.clone();
        edited.title = " New ".into();
        edited.tags = vec!["b +c".into()];
        edited.due = "tom 17:00".into();
        edited.estimate = "1h30m".into();
        edited.notes = "hello\n".into();
        let mods = edited.changes(&original, TODAY).unwrap();
        assert_eq!(mods.title.as_deref(), Some("New"));
        assert_eq!(mods.add_tags, ["c"]);
        assert_eq!(mods.remove_tags, ["a"]);
        let due = mods.due.unwrap().into_option().unwrap();
        assert_eq!(due.date, date(2026, 10, 16));
        assert!(due.time.is_some());
        assert_eq!(mods.estimate, Some(Change::Set(90)));
        assert_eq!(mods.notes.as_deref(), Some("hello"));
        assert_eq!(mods.recur, None, "untouched repeat rule is kept");
        assert_eq!(mods.project, None);
    }

    #[test]
    fn clearing_and_errors() {
        let original = form();
        let mut edited = original.clone();
        edited.project = String::new();
        edited.repeat = String::new();
        edited.priority = "!high".into();
        let mods = edited.changes(&original, TODAY).unwrap();
        assert_eq!(mods.project, Some(Change::Clear));
        assert_eq!(mods.recur, Some(Change::Clear));
        assert_eq!(mods.priority, Priority::new(2));

        for (field, value, expected) in [
            ("title", "", "title:"),
            ("due", "someday-ish", "due:"),
            ("estimate", "soon", "estimate:"),
            ("tags", "9lives", "tags:"),
            ("priority", "meh", "priority:"),
        ] {
            let mut bad = original.clone();
            match field {
                "title" => bad.title = value.into(),
                "due" => bad.due = value.into(),
                "estimate" => bad.estimate = value.into(),
                "tags" => bad.tags = vec![value.into()],
                _ => bad.priority = value.into(),
            }
            let err = bad.changes(&original, TODAY).unwrap_err();
            assert!(err.starts_with(expected), "{field}: {err}");
        }
    }
}
