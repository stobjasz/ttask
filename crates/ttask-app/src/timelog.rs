//! Fixing the time log: entries logged by hand, edited and removed. Each
//! change is one undo step; starting and stopping the timer is not.
//!
//! A log spec is `[duration] [start]`, e.g. `45m`, `1h30m yesterday@14:00`,
//! `2h mon`, `30m 9:15`. A start without a time keeps the time of day of
//! the reference: now for a new entry, the entry's own start when editing.

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use ttask_core::TaskId;
use ttask_core::date::{parse_date, parse_time};
use ttask_core::duration::{format_minutes, parse_minutes};
use ttask_core::{Task, jiff};
use ttask_store::{EventKind, TimeEntry};

use crate::{App, AppError, Result};

const MINUTE_MS: i64 = 60_000;
/// Clock skew allowed before an end counts as "in the future".
const SLACK_MS: i64 = MINUTE_MS;

/// A parsed `[duration] [start]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogSpec {
    pub minutes: Option<u32>,
    pub start: Option<Start>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    At(Date, Time),
    /// A date alone: the time of day comes from the reference.
    On(Date),
}

impl LogSpec {
    pub fn parse(text: &str, today: Date) -> Result<LogSpec> {
        let mut spec = LogSpec::default();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut rest = words.as_slice();
        if let Some((first, tail)) = rest.split_first() {
            if let Some(minutes) = parse_minutes(first) {
                if minutes == 0 {
                    return Err(AppError::Invalid("a time entry needs a length".into()));
                }
                spec.minutes = Some(minutes);
                rest = tail;
            }
        }
        match rest {
            [] => {}
            [start] => spec.start = Some(parse_start(start, today)?),
            _ => {
                return Err(AppError::Invalid(format!(
                    "expected a length and a start like `45m yesterday@14:00`, got `{}`",
                    text.trim()
                )));
            }
        }
        Ok(spec)
    }
}

fn parse_start(word: &str, today: Date) -> Result<Start> {
    let lower = word.to_ascii_lowercase();
    // A bare clock time is today: `9:15`, `2pm`.
    if !lower.contains('@')
        && (lower.contains(':') || lower.ends_with("am") || lower.ends_with("pm"))
    {
        if let Some(time) = parse_time(&lower) {
            return Ok(Start::At(today, time));
        }
    }
    let spec = parse_date(word, today).map_err(|e| AppError::Invalid(e.to_string()))?;
    Ok(match spec.time {
        Some(time) => Start::At(spec.date, time),
        None => Start::On(spec.date),
    })
}

fn to_ms(date: Date, time: Time, tz: &TimeZone) -> Result<i64> {
    date.to_datetime(time)
        .to_zoned(tz.clone())
        .map(|z| z.timestamp().as_millisecond())
        .map_err(|e| AppError::Invalid(format!("time entry: {e}")))
}

fn time_of(ms: i64, tz: &TimeZone) -> Result<(Date, Time)> {
    let z = jiff::Timestamp::from_millisecond(ms)
        .map_err(|e| AppError::Invalid(format!("time entry: {e}")))?
        .to_zoned(tz.clone());
    Ok((z.date(), z.time()))
}

/// Start and end of a new entry: it ends now unless the spec says when.
pub fn resolve_new(spec: &LogSpec, now_ms: i64, tz: &TimeZone) -> Result<(i64, i64)> {
    let minutes = spec
        .minutes
        .ok_or_else(|| AppError::Invalid("how long? e.g. `45m` or `1h30m yesterday`".into()))?;
    let length = i64::from(minutes) * MINUTE_MS;
    let (start, end) = match spec.start {
        None => (now_ms - length, now_ms),
        Some(Start::At(date, time)) => {
            let start = to_ms(date, time, tz)?;
            (start, start + length)
        }
        Some(Start::On(date)) => {
            let (_, now_time) = time_of(now_ms, tz)?;
            let end = to_ms(date, now_time, tz)?;
            (end - length, end)
        }
    };
    check_not_future(end, now_ms)?;
    Ok((start, end))
}

/// New start and end for `entry` (`None`: still running).
pub fn resolve_edit(
    spec: &LogSpec,
    entry: &TimeEntry,
    now_ms: i64,
    tz: &TimeZone,
) -> Result<(i64, Option<i64>)> {
    if spec.minutes.is_none() && spec.start.is_none() {
        return Err(AppError::Invalid(
            "give a new length, a new start, or both (e.g. `45m 9:15`)".into(),
        ));
    }
    let start = match spec.start {
        None => entry.started_at,
        Some(Start::At(date, time)) => to_ms(date, time, tz)?,
        Some(Start::On(date)) => {
            let (_, time) = time_of(entry.started_at, tz)?;
            to_ms(date, time, tz)?
        }
    };
    let end = match (spec.minutes, entry.ended_at) {
        (Some(minutes), _) => Some(start + i64::from(minutes) * MINUTE_MS),
        (None, Some(end)) => Some(start + (end - entry.started_at)),
        (None, None) => None,
    };
    check_not_future(end.unwrap_or(start), now_ms)?;
    Ok((start, end))
}

fn check_not_future(ms: i64, now_ms: i64) -> Result<()> {
    if ms > now_ms + SLACK_MS {
        return Err(AppError::Invalid(
            "that would end in the future; the log is for time already spent".into(),
        ));
    }
    Ok(())
}

/// `09:15–10:00 (45m)`, with the date when it is not `today` (always
/// without one).
pub fn describe(entry: &TimeEntry, now_ms: i64, tz: &TimeZone, today: Option<Date>) -> String {
    let format = |ms: i64, with_date: bool| {
        jiff::Timestamp::from_millisecond(ms)
            .map(|t| {
                let z = t.to_zoned(tz.clone());
                if with_date {
                    z.strftime("%Y-%m-%d %H:%M").to_string()
                } else {
                    z.strftime("%H:%M").to_string()
                }
            })
            .unwrap_or_default()
    };
    let start_day = ttask_core::date_of_ms(entry.started_at, tz);
    let mut text = format(entry.started_at, Some(start_day) != today);
    text.push('–');
    match entry.ended_at {
        Some(end) => {
            let same_day = ttask_core::date_of_ms(end, tz) == start_day;
            text.push_str(&format(end, !same_day));
        }
        None => text.push_str("now"),
    }
    text.push_str(&format!(
        " ({})",
        format_minutes(entry.minutes(now_ms).max(0) as u32)
    ));
    text
}

impl App {
    /// Logs time on `task` from a spec such as `45m` or `1h yesterday@14:00`.
    pub fn log_time(&mut self, task: TaskId, spec: &str) -> Result<TimeEntry> {
        let spec = LogSpec::parse(spec, self.today())?;
        let (start, end) = resolve_new(&spec, self.now_ms(), &self.clock().time_zone())?;
        self.log_time_between(task, start, end)
    }

    pub fn log_time_between(&mut self, task: TaskId, start: i64, end: i64) -> Result<TimeEntry> {
        self.batch(|tx| {
            tx.track(task, EventKind::Updated, |db| {
                db.add_time_entry(task, start, end)
            })
        })
    }

    /// Changes an entry's length and/or start (see [`resolve_edit`]).
    pub fn edit_time_entry(&mut self, id: i64, spec: &str) -> Result<TimeEntry> {
        let spec = LogSpec::parse(spec, self.today())?;
        let entry = self.store().db().time_entry(id)?;
        let (start, end) = resolve_edit(&spec, &entry, self.now_ms(), &self.clock().time_zone())?;
        self.batch(|tx| {
            tx.track(entry.task_id, EventKind::Updated, |db| {
                db.update_time_entry(id, start, end)
            })
        })
    }

    /// Removes entries (one undo step).
    pub fn remove_time_entries(&mut self, ids: &[i64]) -> Result<Vec<TimeEntry>> {
        self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                let task = tx.db.time_entry(id)?.task_id;
                out.push(tx.track(task, EventKind::Updated, |db| db.remove_time_entry(id))?);
            }
            Ok(out)
        })
    }

    /// One task's entries, oldest first.
    pub fn time_entries(&self, task: TaskId) -> Result<Vec<TimeEntry>> {
        self.store().db().task(task)?;
        Ok(self.store().db().time_entries(task)?)
    }

    /// Entries on live tasks overlapping the last `days` days (today
    /// included), oldest first.
    pub fn time_log(&self, days: u32) -> Result<Vec<(TimeEntry, Task)>> {
        let tz = self.clock().time_zone();
        let first = self
            .today()
            .checked_sub(jiff::Span::new().days(i64::from(days.max(1)) - 1))
            .unwrap_or(self.today());
        let from = ttask_core::start_of_day_ms(first, &tz);
        let db = self.store().db();
        let mut out = Vec::new();
        for entry in db.time_entries_between(from, i64::MAX)? {
            if let Some(task) = db.try_task(entry.task_id)? {
                out.push((entry, task));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const TODAY: Date = date(2026, 10, 15);

    fn ms(d: Date, h: i8, m: i8) -> i64 {
        to_ms(d, Time::constant(h, m, 0, 0), &TimeZone::UTC).unwrap()
    }

    #[test]
    fn parses_length_and_start() {
        let s = LogSpec::parse("1h30m yesterday@14:00", TODAY).unwrap();
        assert_eq!(s.minutes, Some(90));
        assert_eq!(
            s.start,
            Some(Start::At(date(2026, 10, 14), Time::constant(14, 0, 0, 0)))
        );
        let s = LogSpec::parse("45m 9:15", TODAY).unwrap();
        assert_eq!(s.start, Some(Start::At(TODAY, Time::constant(9, 15, 0, 0))));
        assert_eq!(
            LogSpec::parse("mon", TODAY).unwrap().start,
            Some(Start::On(date(2026, 10, 19)))
        );
        assert!(LogSpec::parse("0m", TODAY).is_err());
        assert!(LogSpec::parse("45m soon", TODAY).is_err());
        assert!(LogSpec::parse("45m a b", TODAY).is_err());
    }

    #[test]
    fn new_entries_end_now_or_where_told() {
        let now = ms(TODAY, 12, 0);
        let tz = TimeZone::UTC;
        let spec = LogSpec::parse("45m", TODAY).unwrap();
        assert_eq!(
            resolve_new(&spec, now, &tz).unwrap(),
            (ms(TODAY, 11, 15), now)
        );
        let spec = LogSpec::parse("1h yesterday", TODAY).unwrap();
        let y = date(2026, 10, 14);
        assert_eq!(
            resolve_new(&spec, now, &tz).unwrap(),
            (ms(y, 11, 0), ms(y, 12, 0))
        );
        let spec = LogSpec::parse("1h tom@9am", TODAY).unwrap();
        assert!(resolve_new(&spec, now, &tz).is_err(), "future");
        assert!(
            resolve_new(&LogSpec::default(), now, &tz).is_err(),
            "no length"
        );
    }

    #[test]
    fn edits_keep_what_is_not_given() {
        let now = ms(TODAY, 12, 0);
        let tz = TimeZone::UTC;
        let entry = TimeEntry {
            id: 1,
            task_id: 1,
            started_at: ms(TODAY, 8, 0),
            ended_at: Some(ms(TODAY, 9, 0)),
        };
        let edit =
            |text: &str| resolve_edit(&LogSpec::parse(text, TODAY).unwrap(), &entry, now, &tz);
        assert_eq!(
            edit("30m").unwrap(),
            (ms(TODAY, 8, 0), Some(ms(TODAY, 8, 30)))
        );
        assert_eq!(
            edit("10:00").unwrap(),
            (ms(TODAY, 10, 0), Some(ms(TODAY, 11, 0)))
        );
        let y = date(2026, 10, 14);
        assert_eq!(edit("yesterday").unwrap(), (ms(y, 8, 0), Some(ms(y, 9, 0))));
        assert!(edit("11:30").is_err(), "would end in the future");
        let running = TimeEntry {
            ended_at: None,
            ..entry
        };
        let spec = LogSpec::parse("7:00", TODAY).unwrap();
        assert_eq!(
            resolve_edit(&spec, &running, now, &tz).unwrap(),
            (ms(TODAY, 7, 0), None)
        );
    }
}
