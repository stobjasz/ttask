//! Quick-add syntax: tokens are parsed out of the text and the rest is the
//! title. The same grammar drives `ttask edit`, where `none` clears a field
//! and `-+tag` removes a tag.
//!
//! | token | meaning |
//! | --- | --- |
//! | `#work`, `#work/clientA` | project (`#none` clears) |
//! | `+tag`, `tag:x` | add tag; `-+tag` / `-tag:x` removes |
//! | `!0`–`!3`, `!!`, `!low`/`!med`/`!high`/`!urgent` | priority |
//! | `due:fri`, `due:tom@17:00` | due date (`due:none` clears) |
//! | `sched:mon`, `start:mon` | scheduled date |
//! | `every:week`, `every!:3d` | recurrence (`!` = after completion) |
//! | `~30m` | estimate |
//! | `remind:due-1h`, `remind:09:00` | reminder |
//! | `\#literal` | escape |

use jiff::civil::{Date, Time};

use crate::date::{DateSpec, parse_date, parse_time};
use crate::duration::parse_minutes;
use crate::recur::{RecurMode, RecurRule};
use crate::task::Priority;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{token}`: {message}")]
pub struct QuickAddError {
    pub token: String,
    pub message: String,
}

/// Set a field, or clear it (`due:none`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<T> {
    Set(T),
    Clear,
}

impl<T> Change<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Change::Set(v) => Some(v),
            Change::Clear => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderSpec {
    /// Minutes before the due date/time (`remind:due-1h`, `remind:due`).
    BeforeDue(u32),
    /// A time on the due date, or today without one (`remind:09:00`).
    OnDueDayAt(Time),
    /// An absolute date and optional time (`remind:tom@9am`).
    At(DateSpec),
}

/// Default time for date-only due dates and reminders.
pub const DEFAULT_REMINDER_TIME: Time = Time::constant(9, 0, 0, 0);

impl ReminderSpec {
    /// Resolves to a local date-time, given the task's due date.
    pub fn resolve(
        self,
        due: Option<(Date, Option<Time>)>,
        today: Date,
    ) -> Option<jiff::civil::DateTime> {
        match self {
            ReminderSpec::BeforeDue(minutes) => {
                let (date, time) = due?;
                let at = date.to_datetime(time.unwrap_or(DEFAULT_REMINDER_TIME));
                at.checked_sub(jiff::Span::new().minutes(i64::from(minutes)))
                    .ok()
            }
            ReminderSpec::OnDueDayAt(time) => {
                let date = due.map(|(d, _)| d).unwrap_or(today);
                Some(date.to_datetime(time))
            }
            ReminderSpec::At(spec) => Some(
                spec.date
                    .to_datetime(spec.time.unwrap_or(DEFAULT_REMINDER_TIME)),
            ),
        }
    }
}

/// Everything a quick-add or edit string asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mods {
    /// Leftover words; `None` when there were none.
    pub title: Option<String>,
    pub project: Option<Change<Vec<String>>>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
    pub priority: Option<Priority>,
    pub due: Option<Change<DateSpec>>,
    pub sched: Option<Change<Date>>,
    pub estimate: Option<Change<u32>>,
    pub recur: Option<Change<(RecurRule, RecurMode)>>,
    pub reminders: Vec<ReminderSpec>,
    pub clear_reminders: bool,
}

impl Mods {
    pub fn is_empty(&self) -> bool {
        *self == Mods::default()
    }
}

/// Parses quick-add / edit text. `today` resolves relative dates.
pub fn parse_mods(input: &str, today: Date) -> Result<Mods, QuickAddError> {
    let mut mods = Mods::default();
    let mut words: Vec<String> = Vec::new();
    for token in input.split_whitespace() {
        if let Some(literal) = token.strip_prefix('\\') {
            words.push(literal.to_owned());
            continue;
        }
        if !apply_token(token, today, &mut mods)? {
            words.push(token.to_owned());
        }
    }
    if !words.is_empty() {
        mods.title = Some(words.join(" "));
    }
    Ok(mods)
}

/// Returns `Ok(false)` when the token is ordinary title text.
fn apply_token(token: &str, today: Date, mods: &mut Mods) -> Result<bool, QuickAddError> {
    let err = |message: String| QuickAddError {
        token: token.to_owned(),
        message,
    };

    if let Some(path) = token.strip_prefix('#').filter(|p| !p.is_empty()) {
        if matches!(path.to_ascii_lowercase().as_str(), "none" | "inbox") {
            mods.project = Some(Change::Clear);
            return Ok(true);
        }
        let segments: Vec<String> = path.split('/').map(str::to_owned).collect();
        if segments.iter().any(String::is_empty) || segments.len() > 2 {
            return Err(err(
                "projects nest one level: #project or #project/sub".into()
            ));
        }
        mods.project = Some(Change::Set(segments));
        return Ok(true);
    }

    if let Some(tag) = token
        .strip_prefix("-+")
        .or_else(|| token.strip_prefix("-tag:"))
    {
        if !is_tag_name(tag) {
            return Err(err("tags start with a letter".into()));
        }
        mods.remove_tags.push(tag.to_owned());
        return Ok(true);
    }
    if let Some(tag) = token
        .strip_prefix('+')
        .or_else(|| token.strip_prefix("tag:"))
    {
        if is_tag_name(tag) {
            mods.add_tags.push(tag.to_owned());
            return Ok(true);
        }
        if token.starts_with("tag:") {
            return Err(err("tags start with a letter".into()));
        }
        return Ok(false);
    }

    if let Some(p) = token.strip_prefix('!').filter(|p| !p.is_empty()) {
        let priority = match p.to_ascii_lowercase().as_str() {
            "!" => Priority::new(2),
            "!!" => Priority::new(3),
            other => Priority::from_name(other)
                .or_else(|| other.parse::<u8>().ok().and_then(Priority::new)),
        };
        return match priority {
            Some(p) => {
                mods.priority = Some(p);
                Ok(true)
            }
            None if p.chars().all(|c| c.is_ascii_alphanumeric()) && p.len() <= 7 => {
                Err(err("priority is !0-!3 or !low/!med/!high/!urgent".into()))
            }
            None => Ok(false),
        };
    }
    if token == "!" {
        mods.priority = Priority::new(1);
        return Ok(true);
    }

    if let Some(estimate) = token.strip_prefix('~').filter(|e| !e.is_empty()) {
        if estimate == "0" || estimate.eq_ignore_ascii_case("none") {
            mods.estimate = Some(Change::Clear);
            return Ok(true);
        }
        let minutes =
            parse_minutes(estimate).ok_or_else(|| err("estimate looks like ~30m or ~2h".into()))?;
        mods.estimate = Some(Change::Set(minutes));
        return Ok(true);
    }

    let Some((key, value)) = token.split_once(':') else {
        return Ok(false);
    };
    let clears = matches!(value.to_ascii_lowercase().as_str(), "none" | "clear" | "-");
    match key.to_ascii_lowercase().as_str() {
        "due" => {
            mods.due = Some(if clears {
                Change::Clear
            } else {
                Change::Set(parse_date(value, today).map_err(|e| err(e.reason.into()))?)
            });
        }
        "sched" | "start" | "scheduled" => {
            mods.sched = Some(if clears {
                Change::Clear
            } else {
                Change::Set(
                    parse_date(value, today)
                        .map_err(|e| err(e.reason.into()))?
                        .date,
                )
            });
        }
        "every" | "every!" => {
            mods.recur = Some(if clears {
                Change::Clear
            } else {
                let rule = RecurRule::from_shorthand(value).map_err(|e| err(e.reason))?;
                let mode = if key.ends_with('!') {
                    RecurMode::Completion
                } else {
                    RecurMode::Due
                };
                Change::Set((rule, mode))
            });
        }
        "remind" => {
            if clears {
                mods.clear_reminders = true;
            } else {
                mods.reminders.push(
                    parse_reminder(value, today).ok_or_else(|| {
                        err("reminder looks like due-1h, 09:00 or tom@9am".into())
                    })?,
                );
            }
        }
        "est" | "estimate" => {
            mods.estimate = Some(if clears {
                Change::Clear
            } else {
                Change::Set(
                    parse_minutes(value)
                        .ok_or_else(|| err("estimate looks like 30m or 2h".into()))?,
                )
            });
        }
        "project" => {
            return apply_token(&format!("#{value}"), today, mods);
        }
        "priority" | "pri" => {
            return apply_token(&format!("!{value}"), today, mods);
        }
        _ => return Ok(false),
    }
    if value.is_empty() {
        return Err(err("missing value".into()));
    }
    Ok(true)
}

fn parse_reminder(value: &str, today: Date) -> Option<ReminderSpec> {
    let lower = value.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("due") {
        if rest.is_empty() {
            return Some(ReminderSpec::BeforeDue(0));
        }
        return parse_minutes(rest.strip_prefix('-')?).map(ReminderSpec::BeforeDue);
    }
    if let Some(time) = parse_time(&lower) {
        return Some(ReminderSpec::OnDueDayAt(time));
    }
    parse_date(&lower, today).ok().map(ReminderSpec::At)
}

/// Tags start with a letter or `_`, then letters, digits, `_`, `-`.
pub fn is_tag_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 15);

    fn parse(s: &str) -> Mods {
        parse_mods(s, TODAY).unwrap()
    }

    #[test]
    fn full_quick_add() {
        let m = parse("Renew passport #admin +gov !2 due:2026-11-30 ~30m every:year");
        assert_eq!(m.title.as_deref(), Some("Renew passport"));
        assert_eq!(m.project, Some(Change::Set(vec!["admin".to_owned()])));
        assert_eq!(m.add_tags, ["gov"]);
        assert_eq!(m.priority, Priority::new(2));
        assert_eq!(
            m.due.unwrap().into_option().unwrap().date,
            date(2026, 11, 30)
        );
        assert_eq!(m.estimate, Some(Change::Set(30)));
        let (rule, mode) = m.recur.unwrap().into_option().unwrap();
        assert_eq!(rule.to_rrule(), "FREQ=YEARLY");
        assert_eq!(mode, RecurMode::Due);
    }

    #[test]
    fn subproject_and_named_priority() {
        let m = parse("Ship it #work/clientA !urgent");
        assert_eq!(
            m.project,
            Some(Change::Set(vec!["work".to_owned(), "clientA".to_owned()]))
        );
        assert_eq!(m.priority, Priority::new(3));
        assert_eq!(parse("x !high").priority, Priority::new(2));
        assert_eq!(parse("x !!").priority, Priority::new(2));
        assert_eq!(parse("x !!!").priority, Priority::new(3));
        assert_eq!(parse("x !").priority, Priority::new(1));
    }

    #[test]
    fn escapes_and_plain_text_stay_in_title() {
        let m = parse(r"Fix \#123 in C++ see http://x.y/z wow!");
        assert_eq!(
            m.title.as_deref(),
            Some("Fix #123 in C++ see http://x.y/z wow!")
        );
        assert!(m.add_tags.is_empty());
    }

    #[test]
    fn clearing_and_removing_for_edit() {
        let m = parse("due:none sched:clear #none -+later every:none ~0 remind:none");
        assert_eq!(m.due, Some(Change::Clear));
        assert_eq!(m.sched, Some(Change::Clear));
        assert_eq!(m.project, Some(Change::Clear));
        assert_eq!(m.remove_tags, ["later"]);
        assert_eq!(m.recur, Some(Change::Clear));
        assert_eq!(m.estimate, Some(Change::Clear));
        assert!(m.clear_reminders);
        assert_eq!(m.title, None);
    }

    #[test]
    fn completion_mode_recurrence() {
        let (rule, mode) = parse("Water plants every!:3d")
            .recur
            .unwrap()
            .into_option()
            .unwrap();
        assert_eq!(rule.to_rrule(), "FREQ=DAILY;INTERVAL=3");
        assert_eq!(mode, RecurMode::Completion);
    }

    #[test]
    fn reminders() {
        let m = parse("x remind:due-1h remind:09:00 remind:tom@8am remind:due");
        assert_eq!(m.reminders.len(), 4);
        assert_eq!(m.reminders[0], ReminderSpec::BeforeDue(60));
        assert_eq!(m.reminders[3], ReminderSpec::BeforeDue(0));
        let due = Some((date(2026, 10, 20), Some(Time::constant(17, 0, 0, 0))));
        assert_eq!(
            m.reminders[0].resolve(due, TODAY),
            Some(date(2026, 10, 20).at(16, 0, 0, 0))
        );
        assert_eq!(
            m.reminders[1].resolve(None, TODAY),
            Some(TODAY.at(9, 0, 0, 0))
        );
    }

    #[test]
    fn errors_name_the_token() {
        let e = parse_mods("x due:someday", TODAY).unwrap_err();
        assert_eq!(e.token, "due:someday");
        assert!(parse_mods("x #a/b/c", TODAY).is_err());
        assert!(parse_mods("x !9", TODAY).is_err());
        assert!(parse_mods("x every:fortnight", TODAY).is_err());
        assert!(parse_mods("x ~abc", TODAY).is_err());
    }
}
