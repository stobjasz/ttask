//! Reports: tasks completed per week, tracked time per project, and how
//! many tasks were overdue at the end of each week.

use jiff::ToSpan;
use jiff::civil::Date;
use serde::Serialize;
use ttask_core::date::start_of_week;
use ttask_core::{jiff, start_of_day_ms};

use crate::{App, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeekCount {
    /// ISO week, `2026-W42`.
    pub week: String,
    /// Its Monday.
    pub start: Date,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectTime {
    /// `None` is the Inbox (tasks without a project).
    pub project: Option<String>,
    pub minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub completed: Vec<WeekCount>,
    /// Since `time_since`, most time first.
    pub time: Vec<ProjectTime>,
    pub time_since: Date,
    /// Open tasks past their due date at the end of each week (today for
    /// the current one). Based on current due dates: a due date moved later
    /// also moves the task out of earlier weeks.
    pub overdue: Vec<WeekCount>,
}

const DAY_MS: i64 = 86_400_000;

impl App {
    /// The last `weeks` ISO weeks including this one; time is counted over
    /// the same period.
    pub fn report(&self, weeks: u32) -> Result<Report> {
        let weeks = weeks.max(1);
        let today = self.today();
        let tz = self.clock().time_zone();
        let starts: Vec<Date> = (0..weeks)
            .rev()
            .map(|w| start_of_week(today) - (i64::from(w) * 7).days())
            .collect();
        let first = starts[0];
        let start_ms = |d: Date| start_of_day_ms(d, &tz);
        let label = |d: Date| {
            let w = d.iso_week_date();
            format!("{}-W{:02}", w.year(), w.week())
        };
        let db = self.store().db();

        let done = db.completions_since(start_ms(first))?;
        let completed = starts
            .iter()
            .map(|&s| {
                let (from, to) = (start_ms(s), start_ms(s) + 7 * DAY_MS);
                WeekCount {
                    week: label(s),
                    start: s,
                    count: done.iter().filter(|&&t| t >= from && t < to).count() as u32,
                }
            })
            .collect();

        let now = self.now_ms();
        let mut by_project: Vec<ProjectTime> = Vec::new();
        for row in db.time_rows_since(start_ms(first))? {
            let from = row.started_at.max(start_ms(first));
            let to = row.ended_at.unwrap_or(now);
            let minutes = ((to - from).max(0) / 60_000) as u32;
            match by_project.iter_mut().find(|p| p.project == row.project) {
                Some(p) => p.minutes += minutes,
                None => by_project.push(ProjectTime {
                    project: row.project,
                    minutes,
                }),
            }
        }
        by_project.retain(|p| p.minutes > 0);
        by_project.sort_by(|a, b| b.minutes.cmp(&a.minutes).then(a.project.cmp(&b.project)));

        let due = db.due_rows()?;
        let overdue = starts
            .iter()
            .map(|&s| {
                // Overdue as of the last day of the week (or today).
                let day = (s + 6.days()).min(today);
                let end = start_ms(day) + DAY_MS;
                let day_text = day.to_string();
                let count = due
                    .iter()
                    .filter(|r| {
                        r.due_date.as_str() < day_text.as_str()
                            && r.created_at < end
                            && r.closed_at.is_none_or(|c| c >= end)
                    })
                    .count() as u32;
                WeekCount {
                    week: label(s),
                    start: s,
                    count,
                }
            })
            .collect();

        Ok(Report {
            completed,
            time: by_project,
            time_since: first,
            overdue,
        })
    }
}

impl Report {
    /// Plain-text rendering with bar charts, shared by `ttask report` and
    /// the TUI. Section titles are the lines that do not start with a space.
    pub fn to_lines(&self, bar_width: usize) -> Vec<String> {
        let mut out = Vec::new();
        let bar = |n: u32, max: u32| {
            let len = if max == 0 {
                0
            } else {
                (n as usize * bar_width).div_ceil(max as usize)
            };
            "█".repeat(len)
        };
        let weeks = |out: &mut Vec<String>, rows: &[WeekCount]| {
            let max = rows.iter().map(|r| r.count).max().unwrap_or(0);
            for r in rows {
                out.push(format!(
                    "  {}  {:<w$} {}",
                    r.week,
                    bar(r.count, max),
                    r.count,
                    w = bar_width
                ));
            }
        };
        out.push("Completed per week".into());
        weeks(&mut out, &self.completed);
        out.push(String::new());
        out.push(format!("Time per project since {}", self.time_since));
        if self.time.is_empty() {
            out.push("  nothing tracked (T in the TUI, or `ttask timer start`)".into());
        }
        let max = self.time.iter().map(|p| p.minutes).max().unwrap_or(0);
        let name_width = self
            .time
            .iter()
            .map(|p| p.project.as_deref().unwrap_or("Inbox").chars().count())
            .max()
            .unwrap_or(0);
        for p in &self.time {
            out.push(format!(
                "  {:<nw$}  {:<w$} {}",
                p.project.as_deref().unwrap_or("Inbox"),
                bar(p.minutes, max),
                ttask_core::duration::format_minutes(p.minutes),
                nw = name_width,
                w = bar_width
            ));
        }
        out.push(String::new());
        out.push("Overdue at the end of each week".into());
        weeks(&mut out, &self.overdue);
        out
    }
}
