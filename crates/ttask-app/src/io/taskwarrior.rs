//! Import from `task export` (Taskwarrior 2.x and 3.x JSON).
//!
//! Taskwarrior keeps a recurring "template" plus generated instances; ttask
//! keeps one open task per series, so only the earliest pending instance of
//! each series is imported (or the template itself when none is pending).
//! Deleted tasks are skipped. The Taskwarrior uuid becomes the ttask uid, so
//! importing the same export again updates instead of duplicating.

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use serde_json::{Map, Value};
use ttask_core::quickadd::is_tag_name;
use ttask_core::recur::{RecurMode, RecurRule};
use ttask_core::{Priority, Status, jiff};

use super::{ImportBatch, ImportTask};
use crate::{AppError, Result};

type Object = Map<String, Value>;

pub fn taskwarrior_import(text: &str, tz: &TimeZone) -> Result<ImportBatch> {
    let records = parse_records(text)?;
    let mut batch = ImportBatch::default();

    // Earliest pending instance per recurring template.
    let mut first_instance: HashMap<String, (Option<i64>, usize)> = HashMap::new();
    for (i, obj) in records.iter().enumerate() {
        if matches!(str_field(obj, "status"), Some("pending" | "waiting")) {
            if let Some(parent) = str_field(obj, "parent") {
                let due = str_field(obj, "due").and_then(parse_tw_date);
                let entry = first_instance.entry(parent.to_owned()).or_insert((due, i));
                if due.is_some() && (entry.0.is_none() || due < entry.0) {
                    *entry = (due, i);
                }
            }
        }
    }
    let templates: HashMap<&str, &Object> = records
        .iter()
        .filter(|o| str_field(o, "status") == Some("recurring"))
        .filter_map(|o| Some((str_field(o, "uuid")?, o)))
        .collect();

    let mut deleted = 0;
    let mut extra_instances = 0;
    for (i, obj) in records.iter().enumerate() {
        let status = str_field(obj, "status").unwrap_or("pending");
        let parent = str_field(obj, "parent");
        match status {
            "deleted" => {
                deleted += 1;
                continue;
            }
            "recurring" => {
                let uuid = str_field(obj, "uuid").unwrap_or_default();
                if first_instance.contains_key(uuid) {
                    continue; // represented by its pending instance
                }
            }
            "pending" | "waiting"
                if parent
                    .is_some_and(|p| first_instance.get(p).map(|(_, idx)| *idx) != Some(i)) =>
            {
                extra_instances += 1;
                continue;
            }
            _ => {}
        }
        let template = parent.and_then(|p| templates.get(p).copied());
        match convert(obj, template, tz, &mut batch.warnings) {
            Some(task) => batch.tasks.push(task),
            None => batch
                .warnings
                .push("skipped a task without uuid or description".into()),
        }
    }
    if deleted > 0 {
        batch
            .warnings
            .push(format!("skipped {deleted} deleted task(s)"));
    }
    if extra_instances > 0 {
        batch.warnings.push(format!(
            "skipped {extra_instances} extra pending instance(s) of recurring tasks; ttask keeps one per series"
        ));
    }
    Ok(batch)
}

/// A JSON array (`task export`) or one object per line (older versions).
fn parse_records(text: &str) -> Result<Vec<Object>> {
    let trimmed = text.trim();
    let bad = |e: serde_json::Error| AppError::Invalid(format!("not a Taskwarrior export: {e}"));
    let values: Vec<Value> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(bad)?
    } else {
        trimmed
            .lines()
            .map(|l| l.trim().trim_end_matches(','))
            .filter(|l| !l.is_empty() && *l != "[" && *l != "]")
            .map(|l| serde_json::from_str(l).map_err(bad))
            .collect::<Result<_>>()?
    };
    values
        .into_iter()
        .map(|v| match v {
            Value::Object(o) => Ok(o),
            other => Err(AppError::Invalid(format!(
                "not a Taskwarrior export: expected task objects, found {other}"
            ))),
        })
        .collect()
}

fn convert(
    obj: &Object,
    template: Option<&Object>,
    tz: &TimeZone,
    warnings: &mut Vec<String>,
) -> Option<ImportTask> {
    let uuid = str_field(obj, "uuid")?;
    let title = str_field(obj, "description")?.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let mut task = ImportTask {
        uid: Some(uuid.to_owned()),
        key: Some(uuid.to_owned()),
        title: title.clone(),
        status: Some(match str_field(obj, "status") {
            Some("completed") => Status::Done,
            _ => Status::Open,
        }),
        priority: match str_field(obj, "priority") {
            Some("H") => Priority::new(2),
            Some("M") => Priority::new(1),
            _ => Priority::new(0),
        }
        .unwrap_or_default(),
        created_at: str_field(obj, "entry").and_then(parse_tw_date),
        updated_at: str_field(obj, "modified").and_then(parse_tw_date),
        completed_at: str_field(obj, "end").and_then(parse_tw_date),
        ..ImportTask::default()
    };

    if let Some(project) = str_field(obj, "project").filter(|p| !p.is_empty()) {
        // ttask nests one level: Home.Garden.Shed → Home/Garden.Shed.
        task.project_path = Some(match project.split_once('.') {
            Some((parent, rest)) => format!("{parent}/{rest}"),
            None => project.to_owned(),
        });
    }
    if let Some(Value::Array(tags)) = obj.get("tags") {
        for tag in tags.iter().filter_map(Value::as_str) {
            if is_tag_name(tag) {
                task.tags.push(tag.to_owned());
            } else {
                warnings.push(format!(
                    "`{title}`: dropped tag `{tag}` (tags start with a letter)"
                ));
            }
        }
    }
    if let Some((date, time)) = str_field(obj, "due").and_then(|d| local(d, tz)) {
        task.due_date = Some(date);
        task.due_time = time;
    }
    task.scheduled_date = str_field(obj, "scheduled")
        .and_then(|d| local(d, tz))
        .map(|(d, _)| d);
    task.wait_until = str_field(obj, "wait")
        .and_then(|d| local(d, tz))
        .map(|(d, _)| d);

    // Annotations become timestamped note lines.
    if let Some(Value::Array(notes)) = obj.get("annotations") {
        let lines: Vec<String> = notes
            .iter()
            .filter_map(|a| {
                let text = a.get("description")?.as_str()?;
                let date = a
                    .get("entry")
                    .and_then(Value::as_str)
                    .and_then(|e| local(e, tz))
                    .map(|(d, _)| format!("{d}: "))
                    .unwrap_or_default();
                Some(format!("{date}{text}"))
            })
            .collect();
        task.notes = lines.join("\n");
    }

    task.blocked_by = match obj.get("depends") {
        Some(Value::Array(ids)) => ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(ids)) => ids
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };

    // Recurrence: pending instances and templates keep the series going;
    // completed instances are past occurrences.
    let recur = str_field(obj, "recur").or_else(|| template.and_then(|t| str_field(t, "recur")));
    if let (Some(recur), true) = (recur, task.status == Some(Status::Open)) {
        match tw_recurrence(recur) {
            Some(mut rule) => {
                let until = str_field(obj, "until")
                    .or_else(|| template.and_then(|t| str_field(t, "until")));
                rule.until = until.and_then(|u| local(u, tz)).map(|(d, _)| d);
                task.recur_rule = Some(rule.to_rrule());
                task.recur_mode = Some(RecurMode::Due);
                task.recur_series = str_field(obj, "parent").map(str::to_owned);
            }
            None => warnings.push(format!(
                "`{title}`: unsupported recurrence `{recur}`; imported once"
            )),
        }
    }
    Some(task)
}

fn str_field<'a>(obj: &'a Object, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

/// `20261015T120000Z` → UTC milliseconds.
fn parse_tw_date(s: &str) -> Option<i64> {
    tw_timestamp(s).map(|t| t.as_millisecond())
}

fn tw_timestamp(s: &str) -> Option<Timestamp> {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 14 {
        return s.parse::<Timestamp>().ok();
    }
    let num = |range: std::ops::Range<usize>| digits.get(range)?.parse::<i16>().ok();
    let date = Date::new(num(0..4)?, num(4..6)? as i8, num(6..8)? as i8).ok()?;
    let time = Time::new(num(8..10)? as i8, num(10..12)? as i8, num(12..14)? as i8, 0).ok()?;
    date.to_datetime(time)
        .to_zoned(TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

/// A Taskwarrior instant as a local date, with a time unless it falls on
/// local midnight or 23:59:59 (Taskwarrior's "end of day").
fn local(s: &str, tz: &TimeZone) -> Option<(Date, Option<Time>)> {
    let zoned = tw_timestamp(s)?.to_zoned(tz.clone());
    let time = zoned.time();
    let date_only = (time.hour(), time.minute()) == (0, 0)
        || (time.hour(), time.minute(), time.second()) == (23, 59, 59);
    Some((
        zoned.date(),
        (!date_only).then(|| Time::new(time.hour(), time.minute(), 0, 0).expect("valid")),
    ))
}

/// Taskwarrior `recur:` values: `daily`, `weekdays`, `weekly`, `biweekly`,
/// `monthly`, `quarterly`, `yearly`, `3d`, `2wks`, `6mo`, `1yr`, …
fn tw_recurrence(value: &str) -> Option<RecurRule> {
    let v = value.trim().to_ascii_lowercase();
    let shorthand = match v.as_str() {
        "daily" | "day" => "day".to_owned(),
        "weekdays" => "weekday".to_owned(),
        "weekly" | "week" | "sennight" => "week".to_owned(),
        "biweekly" | "fortnight" | "fortnightly" => "2w".to_owned(),
        "monthly" | "month" => "month".to_owned(),
        "bimonthly" => "2m".to_owned(),
        "quarterly" | "quarter" => "3m".to_owned(),
        "semiannual" => "6m".to_owned(),
        "yearly" | "annual" | "year" => "year".to_owned(),
        "biannual" | "biyearly" => "2y".to_owned(),
        _ => {
            let split = v.find(|c: char| !c.is_ascii_digit())?;
            let n: u32 = if split == 0 {
                1
            } else {
                v[..split].parse().ok()?
            };
            let (n, unit) = match &v[split..] {
                "d" | "day" | "days" => (n, 'd'),
                "w" | "wk" | "wks" | "week" | "weeks" => (n, 'w'),
                "mo" | "mos" | "mth" | "mths" | "month" | "months" => (n, 'm'),
                "q" | "qtr" | "qtrs" | "quarter" | "quarters" => (n * 3, 'm'),
                "y" | "yr" | "yrs" | "year" | "years" => (n, 'y'),
                _ => return None,
            };
            format!("{n}{unit}")
        }
    };
    RecurRule::from_shorthand(&shorthand).ok()
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const EXPORT: &str = r#"[
{"id":1,"description":"Pay rent","entry":"20261001T090000Z","modified":"20261002T090000Z","project":"Home.Finance","status":"pending","tags":["bills","next"],"priority":"H","due":"20261015T220000Z","uuid":"aaaa-1","urgency":12.5,"annotations":[{"entry":"20261002T090000Z","description":"ask about the deposit"}]},
{"id":0,"description":"Old chore","end":"20261003T100000Z","entry":"20260901T090000Z","status":"completed","uuid":"aaaa-2"},
{"id":0,"description":"Gone","status":"deleted","uuid":"aaaa-3","entry":"20260901T090000Z"},
{"id":0,"description":"Review","status":"recurring","recur":"weekly","uuid":"tmpl-1","due":"20261009T220000Z","entry":"20260901T090000Z"},
{"id":4,"description":"Review","status":"pending","parent":"tmpl-1","recur":"weekly","uuid":"inst-2","due":"20261023T220000Z","entry":"20260901T090000Z"},
{"id":3,"description":"Review","status":"pending","parent":"tmpl-1","recur":"weekly","uuid":"inst-1","due":"20261016T220000Z","entry":"20260901T090000Z"},
{"id":5,"description":"Sign contract","status":"pending","depends":["aaaa-1"],"wait":"20261020T220000Z","scheduled":"20261018T153000Z","uuid":"aaaa-5","entry":"20260901T090000Z","tags":["c++"]}
]"#;

    // Europe-like offset: UTC+2, so 22:00Z is local midnight of the next day.
    fn tz() -> TimeZone {
        TimeZone::fixed(jiff::tz::offset(2))
    }

    #[test]
    fn maps_fields_and_recurrence() {
        let batch = taskwarrior_import(EXPORT, &tz()).unwrap();
        let titles: Vec<_> = batch.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Pay rent", "Old chore", "Review", "Sign contract"]);

        let rent = &batch.tasks[0];
        assert_eq!(rent.uid.as_deref(), Some("aaaa-1"));
        assert_eq!(rent.project_path.as_deref(), Some("Home/Finance"));
        assert_eq!(rent.tags, ["bills", "next"]);
        assert_eq!(rent.priority, Priority::new(2).unwrap());
        assert_eq!(rent.due_date, Some(date(2026, 10, 16)));
        assert_eq!(rent.due_time, None, "local midnight is date-only");
        assert_eq!(rent.notes, "2026-10-02: ask about the deposit");

        let done = &batch.tasks[1];
        assert_eq!(done.status, Some(Status::Done));
        assert!(done.completed_at.is_some());

        let review = &batch.tasks[2];
        assert_eq!(
            review.uid.as_deref(),
            Some("inst-1"),
            "earliest pending instance"
        );
        assert_eq!(review.recur_rule.as_deref(), Some("FREQ=WEEKLY"));
        assert_eq!(review.due_date, Some(date(2026, 10, 17)));

        let contract = &batch.tasks[3];
        assert_eq!(contract.blocked_by, ["aaaa-1"]);
        assert_eq!(contract.wait_until, Some(date(2026, 10, 21)));
        assert_eq!(contract.scheduled_date, Some(date(2026, 10, 18)));
        assert!(contract.tags.is_empty());

        assert!(batch.warnings.iter().any(|w| w.contains("1 deleted")));
        assert!(batch.warnings.iter().any(|w| w.contains("1 extra pending")));
        assert!(batch.warnings.iter().any(|w| w.contains("c++")));
    }

    #[test]
    fn line_delimited_and_recurrence_names() {
        let text = "{\"uuid\":\"u1\",\"description\":\"A\",\"status\":\"pending\"},\n{\"uuid\":\"u2\",\"description\":\"B\",\"status\":\"pending\"}\n";
        assert_eq!(
            taskwarrior_import(text, &TimeZone::UTC)
                .unwrap()
                .tasks
                .len(),
            2
        );
        assert_eq!(
            tw_recurrence("quarterly").unwrap().to_rrule(),
            "FREQ=MONTHLY;INTERVAL=3"
        );
        assert_eq!(
            tw_recurrence("2wks").unwrap().to_rrule(),
            "FREQ=WEEKLY;INTERVAL=2"
        );
        assert_eq!(
            tw_recurrence("weekdays").unwrap().to_rrule(),
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"
        );
        assert_eq!(tw_recurrence("1yr").unwrap().to_rrule(), "FREQ=YEARLY");
        assert!(tw_recurrence("3min").is_none());
        assert!(taskwarrior_import("not json", &TimeZone::UTC).is_err());
    }
}
