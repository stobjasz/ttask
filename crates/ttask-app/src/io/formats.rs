//! CSV, Markdown, todo.txt, Todoist and TickTick conversions.

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::{Date, Time};
use ttask_core::date::{format_time, parse_date, parse_time};
use ttask_core::quickadd::is_tag_name;
use ttask_core::recur::{RecurMode, RecurRule};
use ttask_core::{Priority, Status, Task, jiff};

use super::{ImportBatch, ImportTask};
use crate::views::arrange_tree;
use crate::{App, AppError, Result};

const CSV_COLUMNS: [&str; 20] = [
    "id",
    "uid",
    "title",
    "status",
    "priority",
    "project",
    "tags",
    "due_date",
    "due_time",
    "scheduled_date",
    "estimate_min",
    "recur_rule",
    "recur_mode",
    "parent_uid",
    "created_at",
    "updated_at",
    "completed_at",
    "wait_until",
    "someday",
    "notes",
];

fn csv_err(e: csv::Error) -> AppError {
    AppError::Invalid(format!("CSV: {e}"))
}

fn all_tasks(app: &App) -> Result<Vec<Task>> {
    let db = app.store().db();
    db.all_task_ids()?
        .into_iter()
        .map(|id| Ok(db.task(id)?))
        .collect()
}

fn timestamp(ms: i64) -> String {
    Timestamp::from_millisecond(ms)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

fn parse_timestamp(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Some(ts.as_millisecond());
    }
    // TickTick style `2026-10-15T08:00:00+0000`: insert the offset colon.
    if s.len() > 5 {
        let (head, tail) = s.split_at(s.len() - 2);
        if let Ok(ts) = format!("{head}:{tail}").parse::<Timestamp>() {
            return Some(ts.as_millisecond());
        }
    }
    let date: Date = s.get(..10)?.parse().ok()?;
    Some(
        date.to_zoned(jiff::tz::TimeZone::UTC)
            .ok()?
            .timestamp()
            .as_millisecond(),
    )
}

/// `2026-10-15`, `2026/10/15`, `2026-10-15T00:00:00+0000`, or a quick-add
/// phrase (`tomorrow`).
fn flexible_date(s: &str, today: Date) -> Option<(Date, Option<Time>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let normalized = s.replace('/', "-");
    if let Some(date) = normalized.get(..10).and_then(|d| d.parse::<Date>().ok()) {
        let time = normalized
            .get(11..16)
            .filter(|t| *t != "00:00")
            .and_then(parse_time);
        return Some((date, time));
    }
    parse_date(s, today).ok().map(|spec| (spec.date, spec.time))
}

fn parse_status(s: &str) -> Option<Status> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "x" | "true" | "yes" | "1" | "2" | "done" | "completed" | "complete" => Some(Status::Done),
        "false" | "no" | "0" | "open" | "todo" => Some(Status::Open),
        other => other.parse().ok(),
    }
}

fn parse_priority(s: &str) -> Priority {
    let s = s.trim().to_ascii_lowercase();
    Priority::from_name(&s)
        .or_else(|| s.parse::<u8>().ok().and_then(Priority::new))
        .or_else(|| match s.as_str() {
            "a" => Priority::new(3),
            "b" => Priority::new(2),
            "c" => Priority::new(1),
            _ => None,
        })
        .unwrap_or_default()
}

fn split_tags(s: &str) -> Vec<String> {
    s.split([' ', ',', ';'])
        .map(|t| t.trim().trim_start_matches(['+', '#', '@']))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------- CSV

/// All live tasks as CSV with the columns in [`CSV_COLUMNS`].
pub fn csv_export(app: &App) -> Result<String> {
    let tasks = all_tasks(app)?;
    let uid_of: HashMap<i64, String> = tasks.iter().map(|t| (t.id, t.uid.clone())).collect();
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(CSV_COLUMNS).map_err(csv_err)?;
    for t in &tasks {
        w.write_record([
            t.id.to_string(),
            t.uid.clone(),
            t.title.clone(),
            t.status.as_str().to_owned(),
            t.priority.value().to_string(),
            t.project.clone().unwrap_or_default(),
            t.tags.join(" "),
            t.due_date.map(|d| d.to_string()).unwrap_or_default(),
            t.due_time.map(format_time).unwrap_or_default(),
            t.scheduled_date.map(|d| d.to_string()).unwrap_or_default(),
            t.estimate_min.map(|m| m.to_string()).unwrap_or_default(),
            t.recur_rule.clone().unwrap_or_default(),
            t.recur_mode
                .map(|m| m.as_str().to_owned())
                .unwrap_or_default(),
            t.parent_id
                .and_then(|p| uid_of.get(&p).cloned())
                .unwrap_or_default(),
            timestamp(t.created_at),
            timestamp(t.updated_at),
            t.completed_at.map(timestamp).unwrap_or_default(),
            t.wait_until.map(|d| d.to_string()).unwrap_or_default(),
            if t.someday { "1" } else { "" }.to_owned(),
            t.notes.clone(),
        ])
        .map_err(csv_err)?;
    }
    let bytes = w
        .into_inner()
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    Ok(String::from_utf8(bytes).expect("CSV of UTF-8 fields"))
}

/// Field name → accepted header names (compared lowercase without spaces,
/// `_` or `-`).
const CSV_ALIASES: &[(&str, &[&str])] = &[
    ("uid", &["uid"]),
    (
        "title",
        &["title", "task", "name", "summary", "subject", "content"],
    ),
    (
        "notes",
        &["notes", "note", "description", "details", "body"],
    ),
    (
        "project",
        &["project", "list", "projectpath", "folder", "category"],
    ),
    ("tags", &["tags", "tag", "labels", "label", "contexts"]),
    ("priority", &["priority", "pri", "importance"]),
    ("due", &["duedate", "due", "deadline", "date"]),
    ("due_time", &["duetime", "time"]),
    (
        "scheduled",
        &[
            "scheduleddate",
            "scheduled",
            "start",
            "startdate",
            "threshold",
        ],
    ),
    (
        "status",
        &["status", "state", "done", "completed", "complete"],
    ),
    ("estimate", &["estimatemin", "estimate", "duration"]),
    (
        "recur",
        &["recurrule", "recurrence", "repeat", "rrule", "recur"],
    ),
    ("recur_mode", &["recurmode"]),
    ("wait", &["waituntil", "wait"]),
    ("someday", &["someday"]),
    ("parent", &["parentuid", "parent"]),
    ("created", &["createdat", "created", "createdtime"]),
    ("updated", &["updatedat", "updated", "modified"]),
    (
        "completed_at",
        &["completedat", "completedtime", "completiondate"],
    ),
];

fn normalize_header(h: &str) -> String {
    h.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Imports CSV with headers. `mapping` overrides header detection, e.g.
/// `[("title", "Name"), ("due", "Deadline")]`.
pub fn csv_import(text: &str, mapping: &[(String, String)], today: Date) -> Result<ImportBatch> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::Fields)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(csv_err)?
        .iter()
        .map(str::to_owned)
        .collect();
    let mut column: HashMap<&str, usize> = HashMap::new();
    for (field, aliases) in CSV_ALIASES {
        if let Some(i) = headers
            .iter()
            .position(|h| aliases.contains(&normalize_header(h).as_str()))
        {
            column.entry(field).or_insert(i);
        }
    }
    // Explicit mapping wins.
    for (field, header) in mapping {
        let Some((known, _)) = CSV_ALIASES.iter().find(|(f, _)| f == field) else {
            return Err(AppError::Invalid(format!(
                "unknown field `{field}` in --map"
            )));
        };
        let i = headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(header))
            .ok_or_else(|| AppError::Invalid(format!("no column named `{header}`")))?;
        column.insert(known, i);
    }
    if !column.contains_key("title") {
        return Err(AppError::Invalid(format!(
            "no title column found among {headers:?}; use --map title=<column>"
        )));
    }
    let mut batch = ImportBatch::default();
    for (row_index, record) in reader.records().enumerate() {
        let record = record.map_err(csv_err)?;
        let get =
            |field: &str| -> &str { column.get(field).and_then(|&i| record.get(i)).unwrap_or("") };
        let mut task = ImportTask {
            uid: Some(get("uid"))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            title: get("title").to_owned(),
            notes: get("notes").to_owned(),
            project_path: Some(get("project").trim_start_matches('#'))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            tags: split_tags(get("tags")),
            priority: parse_priority(get("priority")),
            status: parse_status(get("status")),
            parent: Some(get("parent"))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            created_at: parse_timestamp(get("created")),
            updated_at: parse_timestamp(get("updated")),
            completed_at: parse_timestamp(get("completed_at")),
            estimate_min: ttask_core::duration::parse_minutes(get("estimate")),
            ..ImportTask::default()
        };
        if let Some((date, time)) = flexible_date(get("due"), today) {
            task.due_date = Some(date);
            task.due_time = parse_time(get("due_time")).or(time);
        } else if !get("due").is_empty() {
            batch.warnings.push(format!(
                "row {}: unreadable due date `{}`",
                row_index + 2,
                get("due")
            ));
        }
        task.scheduled_date = flexible_date(get("scheduled"), today).map(|(d, _)| d);
        task.wait_until = flexible_date(get("wait"), today).map(|(d, _)| d);
        task.someday = matches!(
            get("someday").to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "x"
        );
        let recur = get("recur");
        if !recur.is_empty() {
            match parse_recurrence(recur) {
                Some(rule) => {
                    task.recur_rule = Some(rule.to_rrule());
                    task.recur_mode = get("recur_mode").parse().ok().or(Some(RecurMode::Due));
                }
                None => batch.warnings.push(format!(
                    "row {}: unreadable recurrence `{recur}`",
                    row_index + 2
                )),
            }
        }
        batch.tasks.push(task);
    }
    Ok(batch)
}

/// An RRULE (optionally prefixed `RRULE:`) or an `every:` shorthand.
fn parse_recurrence(s: &str) -> Option<RecurRule> {
    let s = s.trim();
    let s = s.strip_prefix("RRULE:").unwrap_or(s);
    let s = s.strip_prefix("every ").unwrap_or(s);
    s.parse::<RecurRule>()
        .ok()
        .or_else(|| RecurRule::from_shorthand(s).ok())
}

// ----------------------------------------------------------- Markdown

/// A human-readable report grouped by project, open tasks first.
pub fn markdown_export(app: &App) -> Result<String> {
    let today = app.today();
    let tasks = all_tasks(app)?;
    let mut groups: Vec<(String, Vec<Task>)> = Vec::new();
    for task in tasks {
        let key = task.project.clone().unwrap_or_default();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(task),
            None => groups.push((key, vec![task])),
        }
    }
    groups.sort_by(|a, b| {
        (!a.0.is_empty(), a.0.to_lowercase()).cmp(&(!b.0.is_empty(), b.0.to_lowercase()))
    });
    let mut out = format!("# ttask export ({today})\n");
    for (project, mut tasks) in groups {
        let heading = if project.is_empty() {
            "Inbox"
        } else {
            &project
        };
        out.push_str(&format!("\n## {heading}\n\n"));
        tasks.sort_by_key(|t| {
            (
                !t.is_open(),
                t.due_date.is_none(),
                t.due_date,
                std::cmp::Reverse(t.priority),
            )
        });
        for (task, depth) in arrange_tree(tasks) {
            let indent = "  ".repeat(depth as usize);
            let check = if task.is_open() { " " } else { "x" };
            let mut meta = Vec::new();
            if let Some(due) = task.due_date {
                let time = task
                    .due_time
                    .map(|t| format!(" {}", format_time(t)))
                    .unwrap_or_default();
                meta.push(format!("due {due}{time}"));
            }
            if task.priority.value() > 0 {
                meta.push(task.priority.name().to_owned());
            }
            if task.someday {
                meta.push("someday".to_owned());
            } else if let Some(wait) = task.wait_until {
                meta.push(format!("waiting until {wait}"));
            }
            meta.extend(task.tags.iter().map(|t| format!("+{t}")));
            if task.status == Status::Cancelled {
                meta.push("cancelled".into());
            }
            let meta = if meta.is_empty() {
                String::new()
            } else {
                format!(" — {}", meta.join(" · "))
            };
            out.push_str(&format!("{indent}- [{check}] {}{meta}\n", task.title));
            for line in task.notes.lines() {
                out.push_str(&format!("{indent}  > {line}\n"));
            }
        }
    }
    Ok(out)
}

// ----------------------------------------------------------- todo.txt

/// One task per line: `x 2026-10-15 2026-10-01 (A) Title +project @tag due:… uid:…`.
pub fn todotxt_export(app: &App) -> Result<String> {
    let tz = app.clock().time_zone();
    let tasks = all_tasks(app)?;
    let uid_of: HashMap<i64, String> = tasks.iter().map(|t| (t.id, t.uid.clone())).collect();
    let mut out = String::new();
    for t in &tasks {
        let mut parts: Vec<String> = Vec::new();
        let created = ttask_core::date_of_ms(t.created_at, &tz);
        if !t.is_open() {
            parts.push("x".into());
            parts.push(
                ttask_core::date_of_ms(t.completed_at.unwrap_or(t.updated_at), &tz).to_string(),
            );
        } else if let Some(letter) = ["", "C", "B", "A"]
            .get(t.priority.value() as usize)
            .filter(|l| !l.is_empty())
        {
            parts.push(format!("({letter})"));
        }
        parts.push(created.to_string());
        parts.push(t.title.replace(['\n', '\r'], " "));
        if let Some(project) = &t.project {
            parts.push(format!("+{}", project.replace(' ', "_")));
        }
        parts.extend(t.tags.iter().map(|tag| format!("@{tag}")));
        if let Some(due) = t.due_date {
            parts.push(format!("due:{due}"));
        }
        if let Some(time) = t.due_time {
            parts.push(format!("time:{}", format_time(time)));
        }
        if let Some(sched) = t.scheduled_date {
            parts.push(format!("t:{sched}"));
        }
        if t.someday {
            parts.push("wait:someday".into());
        } else if let Some(wait) = t.wait_until {
            parts.push(format!("wait:{wait}"));
        }
        if let Some(rule) = &t.recur_rule {
            let mode = if t.recur_mode == Some(RecurMode::Completion) {
                "!"
            } else {
                ""
            };
            parts.push(format!("rrule{mode}:{rule}"));
        }
        if !t.is_open() && t.priority.value() > 0 {
            parts.push(format!("pri:{}", t.priority.value()));
        }
        if let Some(parent) = t.parent_id.and_then(|p| uid_of.get(&p)) {
            parts.push(format!("parent:{parent}"));
        }
        if t.status == Status::Cancelled {
            parts.push("status:cancelled".into());
        }
        parts.push(format!("uid:{}", t.uid));
        out.push_str(&parts.join(" "));
        out.push('\n');
    }
    Ok(out)
}

pub fn todotxt_import(text: &str, today: Date) -> Result<ImportBatch> {
    let mut batch = ImportBatch::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace().peekable();
        let mut task = ImportTask::default();
        let is_date = |s: &str| s.len() == 10 && s.parse::<Date>().is_ok();
        let to_ms = |d: Date| {
            d.to_zoned(jiff::tz::TimeZone::UTC)
                .ok()
                .map(|z| z.timestamp().as_millisecond())
        };
        if tokens.peek() == Some(&"x") {
            tokens.next();
            task.status = Some(Status::Done);
            if let Some(d) = tokens.peek().copied().filter(|t| is_date(t)) {
                task.completed_at = to_ms(d.parse().expect("checked"));
                tokens.next();
            }
        }
        if let Some(p) = tokens.peek().copied() {
            if p.len() == 3 && p.starts_with('(') && p.ends_with(')') {
                task.priority = parse_priority(&p[1..2]);
                tokens.next();
            }
        }
        if let Some(d) = tokens.peek().copied().filter(|t| is_date(t)) {
            task.created_at = to_ms(d.parse().expect("checked"));
            tokens.next();
        }
        let mut words = Vec::new();
        for token in tokens {
            if let Some(project) = token.strip_prefix('+').filter(|p| !p.is_empty()) {
                if task.project_path.is_none() {
                    task.project_path = Some(project.replace('_', " "));
                    continue;
                }
            }
            if let Some(tag) = token.strip_prefix('@').filter(|t| is_tag_name(t)) {
                task.tags.push(tag.to_owned());
                continue;
            }
            if let Some((key, value)) = token
                .split_once(':')
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            {
                match key {
                    "due" => {
                        task.due_date = flexible_date(value, today).map(|(d, _)| d);
                        continue;
                    }
                    "time" => {
                        task.due_time = parse_time(value);
                        continue;
                    }
                    "t" => {
                        task.scheduled_date = flexible_date(value, today).map(|(d, _)| d);
                        continue;
                    }
                    "wait" if value == "someday" => {
                        task.someday = true;
                        continue;
                    }
                    "wait" => {
                        task.wait_until = flexible_date(value, today).map(|(d, _)| d);
                        continue;
                    }
                    "rrule" | "rrule!" => {
                        task.recur_rule = parse_recurrence(value).map(|r| r.to_rrule());
                        task.recur_mode = Some(if key.ends_with('!') {
                            RecurMode::Completion
                        } else {
                            RecurMode::Due
                        });
                        continue;
                    }
                    "rec" => {
                        task.recur_rule =
                            parse_recurrence(value.trim_start_matches('+')).map(|r| r.to_rrule());
                        task.recur_mode = Some(RecurMode::Due);
                        continue;
                    }
                    "pri" => {
                        task.priority = parse_priority(value);
                        continue;
                    }
                    "uid" => {
                        task.uid = Some(value.to_owned());
                        continue;
                    }
                    "parent" => {
                        task.parent = Some(value.to_owned());
                        continue;
                    }
                    "status" if value == "cancelled" => {
                        task.status = Some(Status::Cancelled);
                        continue;
                    }
                    _ => {}
                }
            }
            words.push(token);
        }
        task.title = words.join(" ");
        if task.title.is_empty() {
            batch.warnings.push(format!("line {}: no title", n + 1));
            continue;
        }
        batch.tasks.push(task);
    }
    Ok(batch)
}

// ------------------------------------------------------------ Todoist

/// Todoist's project CSV export: `TYPE,CONTENT,DESCRIPTION,PRIORITY,INDENT,…,DATE,…`.
/// Sections become subprojects of `project`; indented tasks become subtasks.
pub fn todoist_import(text: &str, project: Option<&str>, today: Date) -> Result<ImportBatch> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(csv_err)?
        .iter()
        .map(|h| h.trim().to_ascii_uppercase())
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let (Some(type_col), Some(content_col)) = (col("TYPE"), col("CONTENT")) else {
        return Err(AppError::Invalid(
            "not a Todoist CSV (expected TYPE and CONTENT columns)".into(),
        ));
    };
    let mut batch = ImportBatch::default();
    let mut section: Option<String> = None;
    let mut last_top_key: Option<String> = None;
    for (n, record) in reader.records().enumerate() {
        let record = record.map_err(csv_err)?;
        let get = |c: Option<usize>| c.and_then(|i| record.get(i)).unwrap_or("").trim();
        match get(Some(type_col)) {
            "section" => {
                section = Some(get(Some(content_col)).replace('/', "-"));
                continue;
            }
            "task" => {}
            _ => continue,
        }
        let content = get(Some(content_col));
        let mut words = Vec::new();
        let mut tags = Vec::new();
        for word in content.split_whitespace() {
            match word.strip_prefix('@').filter(|t| is_tag_name(t)) {
                Some(tag) => tags.push(tag.to_owned()),
                None => words.push(word),
            }
        }
        let key = format!("todoist-{n}");
        let indent: u32 = get(col("INDENT")).parse().unwrap_or(1);
        let mut task = ImportTask {
            key: Some(key.clone()),
            title: words.join(" "),
            notes: get(col("DESCRIPTION")).to_owned(),
            tags,
            // Todoist p1 (4) is the most urgent.
            priority: Priority::new(
                get(col("PRIORITY")).parse::<u8>().unwrap_or(1).clamp(1, 4) - 1,
            )
            .unwrap_or_default(),
            project_path: match (project, &section) {
                (Some(p), Some(s)) => Some(format!("{p}/{s}")),
                (Some(p), None) => Some(p.to_owned()),
                (None, Some(s)) => Some(s.clone()),
                (None, None) => None,
            },
            ..ImportTask::default()
        };
        if indent > 1 {
            task.parent = last_top_key.clone();
        } else {
            last_top_key = Some(key);
        }
        let date = get(col("DATE"));
        if !date.is_empty() {
            if let Some(rest) = date
                .strip_prefix("every ")
                .or_else(|| date.strip_prefix("every! "))
            {
                match RecurRule::from_shorthand(rest) {
                    Ok(rule) => {
                        task.recur_rule = Some(rule.to_rrule());
                        task.recur_mode = Some(if date.starts_with("every!") {
                            RecurMode::Completion
                        } else {
                            RecurMode::Due
                        });
                        let yesterday = today.yesterday().unwrap_or(today);
                        task.due_date = rule.next_after(yesterday, today);
                    }
                    Err(_) => keep_unparsed_date(&mut task, date, &mut batch.warnings),
                }
            } else if let Some((d, t)) = flexible_date(date, today) {
                task.due_date = Some(d);
                task.due_time = t;
            } else {
                keep_unparsed_date(&mut task, date, &mut batch.warnings);
            }
        }
        batch.tasks.push(task);
    }
    Ok(batch)
}

fn keep_unparsed_date(task: &mut ImportTask, date: &str, warnings: &mut Vec<String>) {
    warnings.push(format!(
        "`{}`: kept Todoist date `{date}` in notes",
        task.title
    ));
    if !task.notes.is_empty() {
        task.notes.push_str("\n\n");
    }
    task.notes.push_str(&format!("Todoist date: {date}"));
}

// ----------------------------------------------------------- TickTick

/// TickTick's backup CSV (a few preamble lines, then a header row with
/// `Folder Name`, `List Name`, `Title`, …).
pub fn ticktick_import(text: &str, today: Date) -> Result<ImportBatch> {
    let start = text
        .lines()
        .position(|l| {
            l.contains("\"Title\"") || l.starts_with("Folder Name") || l.contains("List Name")
        })
        .ok_or_else(|| AppError::Invalid("not a TickTick CSV (no header row)".into()))?;
    let body: String = text.lines().skip(start).collect::<Vec<_>>().join("\n");
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(body.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(csv_err)?
        .iter()
        .map(normalize_header)
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let mut batch = ImportBatch::default();
    for record in reader.records() {
        let record = record.map_err(csv_err)?;
        let get = |name: &str| col(name).and_then(|i| record.get(i)).unwrap_or("").trim();
        let title = get("title");
        if title.is_empty() {
            continue;
        }
        let folder = get("foldername");
        let list = get("listname").replace('/', "-");
        let project = match (folder.is_empty(), list.is_empty()) {
            (false, false) => Some(format!("{}/{list}", folder.replace('/', "-"))),
            (true, false) => Some(list),
            _ => None,
        };
        let mut task = ImportTask {
            key: Some(get("taskid"))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            parent: Some(get("parentid"))
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            title: title.to_owned(),
            notes: get("content").to_owned(),
            project_path: project,
            tags: split_tags(get("tags")),
            // TickTick: 0 none, 1 low, 3 medium, 5 high.
            priority: Priority::new(match get("priority") {
                "5" => 3,
                "3" => 2,
                "1" => 1,
                _ => 0,
            })
            .unwrap_or_default(),
            status: match get("status") {
                "1" | "2" => Some(Status::Done),
                "-1" => Some(Status::Cancelled),
                _ => Some(Status::Open),
            },
            created_at: parse_timestamp(get("createdtime")),
            completed_at: parse_timestamp(get("completedtime")),
            ..ImportTask::default()
        };
        if let Some((d, t)) = flexible_date(get("duedate"), today) {
            task.due_date = Some(d);
            task.due_time = t;
        }
        task.scheduled_date = flexible_date(get("startdate"), today).map(|(d, _)| d);
        let repeat = get("repeat");
        if !repeat.is_empty() {
            task.recur_rule = parse_recurrence(repeat).map(|r| r.to_rrule());
            task.recur_mode = Some(RecurMode::Due);
        }
        batch.tasks.push(task);
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 15);

    #[test]
    fn csv_with_aliases_and_mapping() {
        let csv = "Name,List,Labels,Deadline,Done,Notes\n\
                   Pay rent,home,bills urgent,2026-11-01,no,\"line1\nline2\"\n\
                   Old,work,,,yes,\n";
        let batch = csv_import(csv, &[], TODAY).unwrap();
        assert_eq!(batch.tasks.len(), 2);
        let t = &batch.tasks[0];
        assert_eq!(t.title, "Pay rent");
        assert_eq!(t.project_path.as_deref(), Some("home"));
        assert_eq!(t.tags, ["bills", "urgent"]);
        assert_eq!(t.due_date, Some(date(2026, 11, 1)));
        assert_eq!(t.notes, "line1\nline2");
        assert_eq!(batch.tasks[1].status, Some(Status::Done));

        let custom = "Thing,When\nX,tomorrow\n";
        assert!(csv_import(custom, &[], TODAY).is_err(), "no title column");
        let mapped = csv_import(
            custom,
            &[
                ("title".into(), "Thing".into()),
                ("due".into(), "When".into()),
            ],
            TODAY,
        )
        .unwrap();
        assert_eq!(mapped.tasks[0].due_date, Some(date(2026, 10, 16)));
    }

    #[test]
    fn todotxt_parses_standard_line() {
        let batch = todotxt_import(
            "(A) 2026-10-01 Call plumber +home @phone due:2026-10-20 t:2026-10-18 rrule:FREQ=WEEKLY uid:U1\n\
             x 2026-10-14 2026-10-01 Done thing +work\n",
            TODAY,
        )
        .unwrap();
        let t = &batch.tasks[0];
        assert_eq!(t.title, "Call plumber");
        assert_eq!(t.priority, Priority::new(3).unwrap());
        assert_eq!(t.project_path.as_deref(), Some("home"));
        assert_eq!(t.tags, ["phone"]);
        assert_eq!(t.due_date, Some(date(2026, 10, 20)));
        assert_eq!(t.scheduled_date, Some(date(2026, 10, 18)));
        assert_eq!(t.recur_rule.as_deref(), Some("FREQ=WEEKLY"));
        assert_eq!(t.uid.as_deref(), Some("U1"));
        assert_eq!(batch.tasks[1].status, Some(Status::Done));
        assert!(batch.tasks[1].completed_at.is_some());
    }

    #[test]
    fn todoist_sections_indent_and_dates() {
        let csv = "TYPE,CONTENT,DESCRIPTION,PRIORITY,INDENT,AUTHOR,RESPONSIBLE,DATE,DATE_LANG,TIMEZONE\n\
                   task,Plan trip @travel,,4,1,,,2026-10-20,en,UTC\n\
                   task,Book hotel,,1,2,,,,en,UTC\n\
                   section,Ideas,,,,,,,,\n\
                   task,Read more,,1,1,,,every monday,en,UTC\n\
                   task,Weird,,1,1,,,whenever,en,UTC\n";
        let batch = todoist_import(csv, Some("Personal"), TODAY).unwrap();
        assert_eq!(batch.tasks.len(), 4);
        let plan = &batch.tasks[0];
        assert_eq!(plan.title, "Plan trip");
        assert_eq!(plan.tags, ["travel"]);
        assert_eq!(plan.priority, Priority::new(3).unwrap());
        assert_eq!(plan.project_path.as_deref(), Some("Personal"));
        assert_eq!(batch.tasks[1].parent, plan.key);
        let read = &batch.tasks[2];
        assert_eq!(read.project_path.as_deref(), Some("Personal/Ideas"));
        assert_eq!(read.recur_rule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(read.due_date, Some(date(2026, 10, 19)));
        assert!(batch.tasks[3].notes.contains("whenever"));
        assert_eq!(batch.warnings.len(), 1);
    }

    #[test]
    fn ticktick_backup() {
        let csv = "\"Date: 2026-10-15+0000\"\n\"Version: 7.1\"\n\"Status: \n0 Normal\n1 Completed\n2 Archived\"\n\
\"Folder Name\",\"List Name\",\"Title\",\"Kind\",\"Tags\",\"Content\",\"Is Check list\",\"Start Date\",\"Due Date\",\"Reminder\",\"Repeat\",\"Priority\",\"Status\",\"Created Time\",\"Completed Time\",\"Order\",\"Timezone\",\"Is All Day\",\"Is Floating\",\"Column Name\",\"Column Order\",\"View Mode\",\"taskId\",\"parentId\"\n\
\"\",\"Work\",\"Ship it\",\"TEXT\",\"q4, big\",\"notes here\",\"N\",\"\",\"2026-10-20T00:00:00+0000\",\"\",\"RRULE:FREQ=WEEKLY;INTERVAL=1\",\"5\",\"0\",\"2026-10-01T08:00:00+0000\",\"\",\"0\",\"UTC\",\"true\",\"false\",\"\",\"\",\"list\",\"1\",\"\"\n\
\"\",\"Work\",\"Sub\",\"TEXT\",\"\",\"\",\"N\",\"\",\"\",\"\",\"\",\"0\",\"2\",\"2026-10-01T08:00:00+0000\",\"2026-10-02T08:00:00+0000\",\"0\",\"UTC\",\"true\",\"false\",\"\",\"\",\"list\",\"2\",\"1\"\n";
        let batch = ticktick_import(csv, TODAY).unwrap();
        assert_eq!(batch.tasks.len(), 2);
        let t = &batch.tasks[0];
        assert_eq!(t.title, "Ship it");
        assert_eq!(t.project_path.as_deref(), Some("Work"));
        assert_eq!(t.tags, ["q4", "big"]);
        assert_eq!(t.priority, Priority::new(3).unwrap());
        assert_eq!(t.due_date, Some(date(2026, 10, 20)));
        assert_eq!(t.recur_rule.as_deref(), Some("FREQ=WEEKLY"));
        assert!(t.created_at.is_some());
        assert_eq!(batch.tasks[1].parent.as_deref(), Some("1"));
        assert_eq!(batch.tasks[1].status, Some(Status::Done));
    }
}
