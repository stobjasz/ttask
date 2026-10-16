//! Snoozing: "remind me again in 15 minutes / tomorrow morning".

use jiff::civil::Date;
use jiff::tz::TimeZone;
use ttask_core::date::{parse_date, parse_time};
use ttask_core::quickadd::DEFAULT_REMINDER_TIME;
use ttask_core::{Task, TaskId, jiff};
use ttask_store::{EventKind, StoreError};

use crate::{App, AppError, Result};

/// When to remind again: a length from now (`15m`, `1h30m`, `2d`, `1w`; a
/// day is 24 hours here), a clock time (`18:00`, `6pm`: today, or tomorrow
/// once it has passed) or a date (`tom`, `mon@8am`; 09:00 without a time).
/// Returns Unix milliseconds, always in the future.
pub fn parse_snooze(text: &str, today: Date, now_ms: i64, tz: &TimeZone) -> Result<i64> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Err(AppError::Invalid(
            "snooze for how long? e.g. 15m, 1h, 18:00 or tom@9am".into(),
        ));
    }
    if let Some(ms) = parse_length(&text) {
        return Ok(now_ms + ms);
    }
    let local = |date: Date, time| {
        date.to_datetime(time)
            .to_zoned(tz.clone())
            .map(|z| z.timestamp().as_millisecond())
            .map_err(|e| AppError::Invalid(format!("snooze: {e}")))
    };
    let clock_time = text.contains(':') || text.ends_with("am") || text.ends_with("pm");
    let at = if clock_time && !text.contains('@') {
        let time = parse_time(&text).ok_or_else(|| {
            AppError::Invalid(format!(
                "cannot understand `{text}`; try 15m, 18:00 or tom@9am"
            ))
        })?;
        let at = local(today, time)?;
        if at > now_ms {
            at
        } else {
            local(today.tomorrow().unwrap_or(today), time)?
        }
    } else {
        let spec = parse_date(&text, today).map_err(|e| AppError::Invalid(e.to_string()))?;
        local(spec.date, spec.time.unwrap_or(DEFAULT_REMINDER_TIME))?
    };
    if at <= now_ms {
        return Err(AppError::Invalid(format!("`{text}` is already past")));
    }
    Ok(at)
}

/// `15m`, `1h30m`, `2d`, `1w` in milliseconds.
fn parse_length(s: &str) -> Option<i64> {
    let mut total: i64 = 0;
    let mut number = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: i64 = number.parse().ok()?;
        number.clear();
        let unit = match c {
            'm' => 60_000,
            'h' => 3_600_000,
            'd' => 86_400_000,
            'w' => 7 * 86_400_000,
            _ => return None,
        };
        total = total.checked_add(n.checked_mul(unit)?)?;
    }
    (number.is_empty() && total > 0).then_some(total)
}

impl App {
    /// Adds a reminder to each task at the time `spec` names (see
    /// [`parse_snooze`]). One undo step. Returns the time and the tasks.
    pub fn snooze(&mut self, ids: &[TaskId], spec: &str) -> Result<(i64, Vec<Task>)> {
        let tz = self.clock().time_zone();
        let at = parse_snooze(spec, self.today(), self.now_ms(), &tz)?;
        let tasks = self.batch(|tx| {
            let mut out = Vec::new();
            for &id in ids {
                let task = tx.db.task(id)?;
                if !task.is_open() {
                    return Err(StoreError::Invalid(format!(
                        "task {id} is {}; reminders only fire for open tasks",
                        task.status
                    ))
                    .into());
                }
                tx.track(id, EventKind::Updated, |db| db.add_reminder(id, at))?;
                out.push(task);
            }
            Ok(out)
        })?;
        Ok((at, tasks))
    }

    /// Open tasks whose reminders fired in the last `minutes` minutes, most
    /// recent first: what a "snooze" right after a notification means.
    pub fn recently_reminded(&self, minutes: i64) -> Result<Vec<Task>> {
        let db = self.store().db();
        let mut out: Vec<Task> = Vec::new();
        for reminder in db.fired_reminders_since(self.now_ms() - minutes * 60_000)? {
            if !out.iter().any(|t| t.id == reminder.task_id) {
                out.push(db.task(reminder.task_id)?);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::{Time, date};

    const TODAY: Date = date(2026, 10, 15);

    fn ms(d: Date, h: i8, m: i8) -> i64 {
        d.to_datetime(Time::constant(h, m, 0, 0))
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_millisecond()
    }

    #[test]
    fn lengths_times_and_dates() {
        let now = ms(TODAY, 12, 0);
        let tz = TimeZone::UTC;
        let p = |s: &str| parse_snooze(s, TODAY, now, &tz);
        assert_eq!(p("15m").unwrap(), ms(TODAY, 12, 15));
        assert_eq!(p("1h30m").unwrap(), ms(TODAY, 13, 30));
        assert_eq!(p("2d").unwrap(), ms(date(2026, 10, 17), 12, 0));
        assert_eq!(p("18:00").unwrap(), ms(TODAY, 18, 0));
        assert_eq!(p("6pm").unwrap(), ms(TODAY, 18, 0));
        assert_eq!(
            p("9am").unwrap(),
            ms(date(2026, 10, 16), 9, 0),
            "rolls over"
        );
        assert_eq!(p("tom").unwrap(), ms(date(2026, 10, 16), 9, 0));
        assert_eq!(p("mon@8am").unwrap(), ms(date(2026, 10, 19), 8, 0));
        assert!(p("").is_err());
        assert!(p("today@10:00").is_err(), "past");
        assert!(p("later").is_err());
    }
}
