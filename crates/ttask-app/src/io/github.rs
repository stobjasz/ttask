//! Import GitHub issues from a JSON file; ttask itself never goes online.
//!
//! Two shapes are accepted: `gh issue list --state all --json
//! number,title,body,url,state,stateReason,labels,milestone,createdAt,updatedAt,closedAt`
//! (camelCase) and the REST API's `GET /repos/{owner}/{repo}/issues`
//! (snake_case; pull requests in it are skipped). The issue URL becomes the
//! ttask uid and the first line of the notes, so importing again updates the
//! same tasks and `ttask open` jumps to the issue.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde_json::{Map, Value};
use ttask_core::quickadd::is_tag_name;
use ttask_core::{Status, jiff};

use super::{ImportBatch, ImportTask};
use crate::{AppError, Result};

type Object = Map<String, Value>;

/// `project` overrides the default project, the repository name.
pub fn github_import(text: &str, project: Option<&str>, tz: &TimeZone) -> Result<ImportBatch> {
    let bad = |msg: String| AppError::Invalid(format!("not a GitHub issues export: {msg}"));
    let value: Value = serde_json::from_str(text.trim()).map_err(|e| bad(e.to_string()))?;
    let records = match value {
        Value::Array(items) => items,
        Value::Object(obj) => vec![Value::Object(obj)],
        _ => return Err(bad("expected a JSON array of issues".into())),
    };
    let mut batch = ImportBatch::default();
    let mut pulls = 0;
    for record in records {
        let Value::Object(obj) = record else {
            return Err(bad("expected a JSON array of issues".into()));
        };
        if obj.contains_key("pull_request") {
            pulls += 1;
            continue;
        }
        match convert(&obj, project, tz, &mut batch.warnings) {
            Some(task) => batch.tasks.push(task),
            None => batch
                .warnings
                .push("skipped an issue without a title or URL".into()),
        }
    }
    if pulls > 0 {
        batch
            .warnings
            .push(format!("skipped {pulls} pull request(s)"));
    }
    Ok(batch)
}

fn convert(
    obj: &Object,
    project: Option<&str>,
    tz: &TimeZone,
    warnings: &mut Vec<String>,
) -> Option<ImportTask> {
    let url = field(obj, &["html_url", "url"]).filter(|u| u.contains("://"))?;
    let title = field(obj, &["title"])?.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let closed = field(obj, &["state"]).is_some_and(|s| s.eq_ignore_ascii_case("closed"));
    let not_planned = field(obj, &["stateReason", "state_reason"])
        .is_some_and(|r| r.eq_ignore_ascii_case("not_planned"));
    let body = field(obj, &["body"]).unwrap_or("").trim();
    let mut task = ImportTask {
        uid: Some(url.to_owned()),
        key: Some(url.to_owned()),
        project_path: project.map(str::to_owned).or_else(|| repo_name(url)),
        title: title.clone(),
        notes: if body.is_empty() {
            url.to_owned()
        } else {
            format!("{url}\n\n{body}")
        },
        status: Some(match (closed, not_planned) {
            (false, _) => Status::Open,
            (true, false) => Status::Done,
            (true, true) => Status::Cancelled,
        }),
        created_at: timestamp(obj, &["createdAt", "created_at"]),
        updated_at: timestamp(obj, &["updatedAt", "updated_at"]),
        completed_at: closed
            .then(|| timestamp(obj, &["closedAt", "closed_at"]))
            .flatten(),
        ..ImportTask::default()
    };
    if let Some(Value::Array(labels)) = obj.get("labels") {
        for label in labels {
            let name = match label {
                Value::String(name) => name.as_str(),
                Value::Object(l) => l.get("name").and_then(Value::as_str).unwrap_or(""),
                _ => "",
            };
            match label_tag(name) {
                Some(tag) if !task.tags.contains(&tag) => task.tags.push(tag),
                Some(_) => {}
                None if name.is_empty() => {}
                None => warnings.push(format!("`{title}`: dropped label `{name}`")),
            }
        }
    }
    if let Some(Value::Object(milestone)) = obj.get("milestone") {
        task.due_date = field(milestone, &["dueOn", "due_on"]).and_then(|d| local_date(d, tz));
    }
    Some(task)
}

fn field<'a>(obj: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
}

fn timestamp(obj: &Object, keys: &[&str]) -> Option<i64> {
    field(obj, keys)?
        .parse::<Timestamp>()
        .ok()
        .map(|t| t.as_millisecond())
}

/// GitHub stores milestone due dates as UTC instants; keep the local date.
fn local_date(s: &str, tz: &TimeZone) -> Option<Date> {
    Some(s.parse::<Timestamp>().ok()?.to_zoned(tz.clone()).date())
}

/// `https://github.com/owner/repo/issues/12` → `repo`.
fn repo_name(url: &str) -> Option<String> {
    let path = url.split("://").nth(1)?;
    let mut parts = path.split('/').skip(1);
    let (_owner, repo) = (parts.next()?, parts.next()?);
    (!repo.is_empty()).then(|| repo.to_owned())
}

/// Labels become tags: spaces and other separators turn into `-`
/// (`good first issue` → `good-first-issue`).
fn label_tag(label: &str) -> Option<String> {
    let mut tag = String::new();
    for c in label.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            tag.push(c);
        } else if !tag.ends_with('-') {
            tag.push('-');
        }
    }
    let tag = tag.trim_matches('-').to_owned();
    is_tag_name(&tag).then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GH: &str = r#"[
      {"number": 12, "title": "Crash on start", "body": "Steps:\n1. run it",
       "url": "https://github.com/acme/rocket/issues/12", "state": "OPEN",
       "labels": [{"name": "bug"}, {"name": "good first issue"}, {"name": "1.0"}],
       "milestone": {"title": "v1", "dueOn": "2026-11-01T07:00:00Z"},
       "createdAt": "2026-10-01T10:00:00Z", "updatedAt": "2026-10-02T10:00:00Z", "closedAt": null},
      {"number": 9, "title": "Old idea", "body": "", "url": "https://github.com/acme/rocket/issues/9",
       "state": "CLOSED", "stateReason": "NOT_PLANNED", "labels": [],
       "createdAt": "2026-09-01T10:00:00Z", "closedAt": "2026-09-05T10:00:00Z"}
    ]"#;

    #[test]
    fn gh_cli_json() {
        let batch = github_import(GH, None, &TimeZone::UTC).unwrap();
        let [open, closed] = &batch.tasks[..] else {
            panic!("{batch:?}")
        };
        assert_eq!(open.title, "Crash on start");
        assert_eq!(
            open.uid.as_deref(),
            Some("https://github.com/acme/rocket/issues/12")
        );
        assert!(
            open.notes
                .starts_with("https://github.com/acme/rocket/issues/12\n\nSteps:")
        );
        assert_eq!(open.project_path.as_deref(), Some("rocket"));
        assert_eq!(open.tags, ["bug", "good-first-issue"]);
        assert_eq!(open.status, Some(Status::Open));
        assert_eq!(open.due_date, Some(jiff::civil::date(2026, 11, 1)));
        assert_eq!(closed.status, Some(Status::Cancelled));
        assert!(closed.completed_at.is_some());
        assert_eq!(closed.notes, "https://github.com/acme/rocket/issues/9");
        assert!(
            batch.warnings.iter().any(|w| w.contains("`1.0`")),
            "{:?}",
            batch.warnings
        );
    }

    #[test]
    fn rest_api_json_skips_pull_requests() {
        let text = r#"[
          {"number": 3, "title": "Docs", "html_url": "https://github.com/acme/rocket/issues/3",
           "url": "https://api.github.com/repos/acme/rocket/issues/3", "state": "closed",
           "state_reason": "completed", "labels": ["docs"], "closed_at": "2026-10-03T00:00:00Z"},
          {"number": 4, "title": "Fix", "html_url": "https://github.com/acme/rocket/pull/4",
           "state": "open", "pull_request": {}}
        ]"#;
        let batch = github_import(text, Some("work/rocket"), &TimeZone::UTC).unwrap();
        assert_eq!(batch.tasks.len(), 1);
        let task = &batch.tasks[0];
        assert_eq!(
            task.uid.as_deref(),
            Some("https://github.com/acme/rocket/issues/3")
        );
        assert_eq!(task.status, Some(Status::Done));
        assert_eq!(task.project_path.as_deref(), Some("work/rocket"));
        assert_eq!(task.tags, ["docs"]);
        assert!(batch.warnings.iter().any(|w| w.contains("pull request")));
    }

    #[test]
    fn rejects_other_json() {
        assert!(github_import("42", None, &TimeZone::UTC).is_err());
        assert!(github_import("not json", None, &TimeZone::UTC).is_err());
    }
}
