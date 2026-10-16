//! The in-app task editor (`e`): every field on one screen, notes in a
//! multi-line area. `E` still hands the notes to `$EDITOR`.

use ttask_core::date::{format_time, relative_label};
use ttask_core::duration::{format_minutes, parse_minutes};
use ttask_core::form::{
    TaskForm, parse_due, parse_priority, parse_project, parse_repeat, parse_wait, split_tags,
};
use ttask_core::jiff::civil::Date;
use ttask_core::quickadd::{Change, Wait, is_tag_name};
use ttask_core::recur::RecurMode;
use ttask_core::{Task, TaskId};

use crate::input::LineInput;
use crate::textarea::TextArea;

/// Single-line fields, in screen order; the notes come after them.
pub const FIELDS: [&str; 9] = [
    "Title",
    "Project",
    "Tags",
    "Priority",
    "Due",
    "Scheduled",
    "Wait",
    "Estimate",
    "Repeat",
];
/// Focus index of the notes area.
pub const NOTES: usize = FIELDS.len();

pub struct Editor {
    pub task_id: TaskId,
    pub inputs: Vec<LineInput>,
    pub notes: TextArea,
    pub focus: usize,
    pub error: Option<String>,
    /// Esc was pressed once with unsaved changes.
    pub confirm_discard: bool,
    original: TaskForm,
}

impl Editor {
    pub fn new(task: &Task) -> Editor {
        let original = TaskForm::from_task(task);
        let tags = original
            .tags
            .iter()
            .map(|t| format!("+{t}"))
            .collect::<Vec<_>>()
            .join(" ");
        let values = [
            &original.title,
            &original.project,
            &tags,
            &original.priority,
            &original.due,
            &original.scheduled,
            &original.wait,
            &original.estimate,
            &original.repeat,
        ];
        Editor {
            task_id: task.id,
            inputs: values.iter().map(|v| LineInput::new(v)).collect(),
            notes: TextArea::new(&original.notes),
            focus: 0,
            error: None,
            confirm_discard: false,
            original,
        }
    }

    pub fn form(&self) -> TaskForm {
        let text = |i: usize| self.inputs[i].text();
        TaskForm {
            title: text(0),
            project: text(1),
            tags: split_tags(&text(2)),
            priority: text(3),
            due: text(4),
            scheduled: text(5),
            wait: text(6),
            estimate: text(7),
            repeat: text(8),
            notes: self.notes.text(),
        }
    }

    pub fn original(&self) -> &TaskForm {
        &self.original
    }

    pub fn is_dirty(&self) -> bool {
        fn key(f: &TaskForm) -> Vec<String> {
            let mut key: Vec<String> = [
                &f.title,
                &f.project,
                &f.priority,
                &f.due,
                &f.scheduled,
                &f.wait,
                &f.estimate,
                &f.repeat,
            ]
            .iter()
            .map(|s| s.trim().to_owned())
            .collect();
            key.push(f.tags.join(" ").to_lowercase());
            key.push(f.notes.trim_end().to_owned());
            key
        }
        key(&self.form()) != key(&self.original)
    }

    pub fn move_focus(&mut self, delta: isize) {
        let n = FIELDS.len() as isize + 1;
        self.focus = (self.focus as isize + delta).rem_euclid(n) as usize;
    }

    /// Focuses the field an error message starts with (`due: …`).
    pub fn focus_error(&mut self) {
        let Some(err) = &self.error else {
            return;
        };
        let field = err.split(':').next().unwrap_or("").to_ascii_lowercase();
        if let Some(i) = FIELDS.iter().position(|f| f.to_ascii_lowercase() == field) {
            self.focus = i;
        }
    }

    /// What field `i`'s text means right now, or why it is wrong: shown
    /// next to the field while you type.
    pub fn hint(&self, i: usize, today: Date) -> Option<Result<String, String>> {
        let text = self.inputs.get(i)?.text();
        let text = text.trim();
        let date_hint =
            |d: Date| format!("{} · {}", d.strftime("%a %-d %b"), relative_label(d, today));
        Some(match i {
            0 if text.is_empty() => Err("a task needs a title".into()),
            1 => Ok(match parse_project(text) {
                Change::Clear => "Inbox".into(),
                Change::Set(path) => format!("#{}", path.join("/")),
            }),
            2 => {
                let bad = split_tags(text).into_iter().find(|t| !is_tag_name(t))?;
                Err(format!("`{bad}` is not a tag"))
            }
            3 => parse_priority(text).map(|p| p.name().to_owned()),
            4 if !text.is_empty() => parse_due(text, today).map(|spec| {
                let mut s = date_hint(spec.date);
                if let Some(time) = spec.time {
                    s.push_str(&format!(" · {}", format_time(time)));
                }
                s
            }),
            5 if !text.is_empty() => parse_due(text, today).map(|spec| date_hint(spec.date)),
            6 if !text.is_empty() => parse_wait(text, today).map(|w| match w {
                Change::Set(Wait::Until(d)) => format!("hidden until {}", date_hint(d)),
                Change::Set(Wait::Someday) => "parked until you bring it back".into(),
                Change::Clear => "shown in the lists".into(),
            }),
            7 if !text.is_empty() => parse_minutes(text)
                .map(format_minutes)
                .ok_or_else(|| "looks like 30m, 2h or 1h30m".into()),
            8 if !text.is_empty() => parse_repeat(text).map(|(rule, mode)| {
                let mut s = rule.describe();
                if mode == RecurMode::Completion {
                    s.push_str(" after completion");
                }
                s
            }),
            4..=8 => Ok("none".into()),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    use super::*;

    fn task() -> Task {
        Task {
            id: 7,
            uid: "u".into(),
            parent_id: None,
            project_id: None,
            project: Some("home".into()),
            title: "Paint".into(),
            notes: "white".into(),
            status: ttask_core::Status::Open,
            priority: ttask_core::Priority::NONE,
            due_date: None,
            due_time: None,
            scheduled_date: None,
            estimate_min: None,
            recur_rule: None,
            recur_mode: None,
            recur_series: None,
            wait_until: None,
            someday: false,
            tags: vec!["diy".into()],
            sort_order: 0.0,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            subtasks: Default::default(),
            checklist: Default::default(),
            blocked: false,
            blocking: false,
            urgency: 0.0,
        }
    }

    #[test]
    fn starts_clean_and_hints_follow_the_text() {
        let today = ttask_core::jiff::civil::date(2026, 10, 15);
        let mut editor = Editor::new(&task());
        assert!(!editor.is_dirty());
        assert_eq!(editor.inputs[2].text(), "+diy");
        assert_eq!(editor.hint(1, today), Some(Ok("#home".into())));
        assert_eq!(editor.hint(4, today), Some(Ok("none".into())));

        editor.inputs[4] = LineInput::new("tom 9am");
        assert_eq!(
            editor.hint(4, today),
            Some(Ok("Fri 16 Oct · tomorrow · 09:00".into()))
        );
        editor.inputs[7] = LineInput::new("soon");
        assert!(matches!(editor.hint(7, today), Some(Err(_))));
        assert!(editor.is_dirty());

        editor.inputs[7] = LineInput::new("");
        editor.inputs[4] = LineInput::new("");
        editor.notes.handle(KeyEvent::from(KeyCode::Char('!')));
        assert!(editor.is_dirty(), "notes changed");
        editor.move_focus(-1);
        assert_eq!(editor.focus, NOTES);
        editor.error = Some("estimate: bad".into());
        editor.focus_error();
        assert_eq!(editor.focus, 7);
    }
}
