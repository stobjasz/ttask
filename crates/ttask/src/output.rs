//! Human-readable CLI output. Colour only on a terminal, never with
//! `NO_COLOR` or `--no-color`.

use std::io::IsTerminal;

use ttask_core::date::{format_time, relative_label};
use ttask_core::duration::format_minutes;
use ttask_core::jiff::civil::Date;
use ttask_core::recur::RecurRule;
use ttask_core::{Status, Task};

#[derive(Debug, Clone, Copy)]
pub struct Painter {
    color: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Tone {
    Dim,
    Bold,
    Red,
    Yellow,
    Cyan,
    Magenta,
}

impl Painter {
    pub fn new(no_color_flag: bool) -> Painter {
        let color = !no_color_flag
            && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
            && std::io::stdout().is_terminal();
        Painter { color }
    }

    pub fn paint(&self, tone: Tone, text: &str) -> String {
        if !self.color || text.is_empty() {
            return text.to_owned();
        }
        let code = match tone {
            Tone::Dim => "2",
            Tone::Bold => "1",
            Tone::Red => "31",
            Tone::Yellow => "33",
            Tone::Cyan => "36",
            Tone::Magenta => "35",
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

pub fn status_box(status: Status) -> &'static str {
    match status {
        Status::Open => "[ ]",
        Status::Done => "[x]",
        Status::Cancelled => "[-]",
    }
}

/// `  12  [ ] !!  Title ↻  #project +tag  Fri`.
pub fn task_line(task: &Task, depth: u8, today: Date, p: &Painter) -> String {
    let indent = "  ".repeat(depth as usize);
    let id = format!("{:>4}", task.id);
    let marks = format!("{:<3}", task.priority.marks());
    let mut title = task.title.clone();
    if task.recur_rule.is_some() {
        title.push_str(" ↻");
    }
    let title = match task.status {
        Status::Open => title,
        _ => p.paint(Tone::Dim, &title),
    };
    let mut meta = Vec::new();
    if let Some(project) = &task.project {
        meta.push(p.paint(Tone::Cyan, &format!("#{project}")));
    }
    for tag in &task.tags {
        meta.push(p.paint(Tone::Magenta, &format!("+{tag}")));
    }
    if task.subtasks.total > 0 {
        meta.push(format!("[{}/{}]", task.subtasks.done, task.subtasks.total));
    } else if task.checklist.total > 0 {
        meta.push(format!(
            "({}/{})",
            task.checklist.done, task.checklist.total
        ));
    }
    if task.blocked {
        meta.push(p.paint(Tone::Yellow, "blocked"));
    }
    if let Some(due) = due_label(task, today) {
        let tone = if task.is_overdue(today) {
            Tone::Red
        } else if task.due_date == Some(today) {
            Tone::Yellow
        } else {
            Tone::Dim
        };
        meta.push(p.paint(tone, &due));
    }
    let marks = if task.priority.value() >= 2 {
        p.paint(Tone::Red, &marks)
    } else {
        marks
    };
    format!(
        "{} {indent}{} {marks} {title}  {}",
        p.paint(Tone::Dim, &id),
        status_box(task.status),
        meta.join("  ")
    )
    .trim_end()
    .to_owned()
}

pub fn due_label(task: &Task, today: Date) -> Option<String> {
    let date = task.due_date?;
    let mut label = relative_label(date, today);
    if let Some(time) = task.due_time {
        label.push(' ');
        label.push_str(&format_time(time));
    }
    Some(label)
}

pub fn describe_recurrence(task: &Task) -> Option<String> {
    let rule: RecurRule = task.recur_rule.as_ref()?.parse().ok()?;
    let mut text = rule.describe();
    if task.recur_mode == Some(ttask_core::recur::RecurMode::Completion) {
        text.push_str(" after completion");
    }
    Some(text)
}

pub fn estimate(task: &Task) -> Option<String> {
    task.estimate_min.map(format_minutes)
}

/// Local `YYYY-MM-DD HH:MM` for a UTC-ms instant.
pub fn instant(ms: i64, tz: &ttask_core::jiff::tz::TimeZone) -> String {
    ttask_core::jiff::Timestamp::from_millisecond(ms)
        .map(|t| {
            t.to_zoned(tz.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| ms.to_string())
}
