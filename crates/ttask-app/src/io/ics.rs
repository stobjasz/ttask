//! iCalendar export: open tasks with a due date become calendar events, so
//! they show up in any calendar app (subscribe to the file, or import it).
//!
//! Date-only due dates are all-day events; timed ones last the task's
//! estimate (30 minutes by default). Schedule-based recurrences carry their
//! RRULE; unfired reminders become alarms.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use ttask_core::recur::RecurMode;
use ttask_core::{Task, jiff};
use ttask_store::TaskSort;

use crate::{App, Result};

const DEFAULT_MINUTES: i64 = 30;

impl App {
    /// Open tasks with a due date matching `filter` (within the context).
    pub fn ics_tasks(&self, filter: &str) -> Result<Vec<(Task, Vec<i64>)>> {
        let db = self.store().db();
        self.list(filter, "due:any", TaskSort::Smart)?
            .into_iter()
            .map(|task| {
                let alarms = db
                    .reminders_for(task.id)?
                    .into_iter()
                    .filter(|r| r.fired_at.is_none())
                    .map(|r| r.fire_at)
                    .collect();
                Ok((task, alarms))
            })
            .collect()
    }
}

/// Renders a VCALENDAR with one VEVENT per task. `alarms` are UTC ms.
pub fn ics_export(tasks: &[(Task, Vec<i64>)], now_ms: i64, tz: &TimeZone) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//ttask//ttask//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "X-WR-CALNAME:ttask".into(),
    ];
    let stamp = utc_stamp(now_ms);
    for (task, alarms) in tasks {
        let Some(due) = task.due_date else {
            continue;
        };
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!("UID:{}@ttask", task.uid));
        lines.push(format!("DTSTAMP:{stamp}"));
        lines.push(format!("SUMMARY:{}", escape(&task.title)));
        let timed = match task.due_time {
            Some(time) => {
                let start = local_ms(due.to_datetime(time), tz);
                let minutes = task
                    .estimate_min
                    .map(i64::from)
                    .unwrap_or(DEFAULT_MINUTES)
                    .max(5);
                lines.push(format!("DTSTART:{}", utc_stamp(start)));
                lines.push(format!("DTEND:{}", utc_stamp(start + minutes * 60_000)));
                true
            }
            None => {
                lines.push(format!("DTSTART;VALUE=DATE:{}", ics_date(due)));
                let next = due.tomorrow().unwrap_or(due);
                lines.push(format!("DTEND;VALUE=DATE:{}", ics_date(next)));
                false
            }
        };
        let mut description = Vec::new();
        if !task.notes.trim().is_empty() {
            description.push(task.notes.trim().to_owned());
        }
        if let Some(project) = &task.project {
            description.push(format!("Project: {project}"));
        }
        if !task.tags.is_empty() {
            let tags: Vec<String> = task.tags.iter().map(|t| format!("+{t}")).collect();
            description.push(format!("Tags: {}", tags.join(" ")));
        }
        description.push(format!("ttask task {}", task.id));
        lines.push(format!("DESCRIPTION:{}", escape(&description.join("\n"))));
        if !task.tags.is_empty() {
            let tags: Vec<String> = task.tags.iter().map(|t| escape(t)).collect();
            lines.push(format!("CATEGORIES:{}", tags.join(",")));
        }
        // RFC 5545: 1 is highest, 9 lowest, 0 undefined.
        let priority = [0, 5, 3, 1][task.priority.value() as usize];
        if priority > 0 {
            lines.push(format!("PRIORITY:{priority}"));
        }
        if let (Some(rule), true) = (
            &task.recur_rule,
            task.recur_mode != Some(RecurMode::Completion),
        ) {
            lines.push(format!("RRULE:{}", rrule_for(rule, timed)));
        }
        for at in alarms {
            lines.push("BEGIN:VALARM".into());
            lines.push("ACTION:DISPLAY".into());
            lines.push(format!("DESCRIPTION:{}", escape(&task.title)));
            lines.push(format!("TRIGGER;VALUE=DATE-TIME:{}", utc_stamp(*at)));
            lines.push("END:VALARM".into());
        }
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    let mut out = String::new();
    for line in lines {
        out.push_str(&fold(&line));
        out.push_str("\r\n");
    }
    out
}

/// UNTIL must match DTSTART's type: a date for all-day events, a UTC
/// date-time for timed ones.
fn rrule_for(rule: &str, timed: bool) -> String {
    if !timed {
        return rule.to_owned();
    }
    rule.split(';')
        .map(|part| match part.strip_prefix("UNTIL=") {
            Some(date) if date.len() == 8 => format!("UNTIL={date}T235959Z"),
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn local_ms(at: DateTime, tz: &TimeZone) -> i64 {
    at.to_zoned(tz.clone())
        .map(|z| z.timestamp().as_millisecond())
        .unwrap_or_default()
}

fn utc_stamp(ms: i64) -> String {
    Timestamp::from_millisecond(ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn ics_date(date: Date) -> String {
    date.strftime("%Y%m%d").to_string()
}

/// TEXT escaping from RFC 5545 §3.3.11.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

/// Folds lines longer than 75 octets, never splitting a UTF-8 character.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use ttask_core::{Priority, Progress, Status};

    use super::*;

    fn task(title: &str) -> Task {
        Task {
            id: 7,
            uid: "UID1".into(),
            parent_id: None,
            project_id: Some(1),
            project: Some("home".into()),
            title: title.into(),
            notes: "line one\nline; two, three".into(),
            status: Status::Open,
            priority: Priority::new(2).unwrap(),
            due_date: Some(date(2026, 10, 20)),
            due_time: None,
            scheduled_date: None,
            estimate_min: None,
            recur_rule: Some("FREQ=WEEKLY;UNTIL=20261231".into()),
            recur_mode: Some(RecurMode::Due),
            recur_series: None,
            wait_until: None,
            someday: false,
            tags: vec!["bills".into()],
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
    fn all_day_event_with_rrule_and_escaping() {
        let ics = ics_export(&[(task("Pay rent"), vec![])], 0, &TimeZone::UTC);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("UID:UID1@ttask\r\n"));
        assert!(ics.contains("DTSTART;VALUE=DATE:20261020\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20261021\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;UNTIL=20261231\r\n"));
        assert!(ics.contains("PRIORITY:3\r\n"));
        assert!(ics.contains("CATEGORIES:bills\r\n"));
        assert!(ics.contains("line one\\nline\\; two\\, three"));
    }

    #[test]
    fn timed_event_uses_utc_estimate_and_alarm() {
        let mut t = task("Dentist");
        t.due_time = Some(jiff::civil::time(10, 0, 0, 0));
        t.estimate_min = Some(60);
        let tz = TimeZone::fixed(jiff::tz::offset(2));
        let alarm = date(2026, 10, 20)
            .at(7, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond();
        let ics = ics_export(&[(t, vec![alarm])], 0, &tz);
        assert!(ics.contains("DTSTART:20261020T080000Z\r\n"), "{ics}");
        assert!(ics.contains("DTEND:20261020T090000Z\r\n"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;UNTIL=20261231T235959Z\r\n"));
        assert!(ics.contains("TRIGGER;VALUE=DATE-TIME:20261020T070000Z\r\n"));
    }

    #[test]
    fn long_lines_fold_on_char_boundaries() {
        let title = "ä".repeat(60);
        let ics = ics_export(&[(task(&title), vec![])], 0, &TimeZone::UTC);
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "{line}");
        }
        let unfolded = ics.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{title}")));
    }
}
