//! Recurrence rules: an RFC 5545 RRULE subset plus `every:` shorthands.
//!
//! A recurring task is one open row; completing it creates the next instance,
//! whose due date comes from [`next_due`].

use std::fmt;
use std::str::FromStr;

use jiff::ToSpan;
use jiff::civil::{Date, Weekday};
use serde::{Deserialize, Serialize};

use crate::date::{days_between, parse_weekday, weekday_abbrev};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot understand recurrence `{input}`: {reason}")]
pub struct RecurError {
    pub input: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecurMode {
    /// Next due date follows the schedule (`every:`).
    #[default]
    Due,
    /// Next due date counts from the completion date (`every!:`).
    Completion,
}

impl RecurMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RecurMode::Due => "due",
            RecurMode::Completion => "completion",
        }
    }
}

impl FromStr for RecurMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "due" => Ok(RecurMode::Due),
            "completion" => Ok(RecurMode::Completion),
            other => Err(format!("unknown recurrence mode `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurRule {
    pub freq: Freq,
    pub interval: u32,
    pub by_day: Vec<Weekday>,
    /// 1..=31, or negative counting from the month's end (-1 = last day).
    pub by_month_day: Vec<i8>,
    /// With `by_day` in a monthly rule: 2 = second, -1 = last.
    pub by_set_pos: Option<i8>,
    pub until: Option<Date>,
    /// Occurrences left including the current one.
    pub count: Option<u32>,
}

impl RecurRule {
    fn new(freq: Freq) -> RecurRule {
        RecurRule {
            freq,
            interval: 1,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_set_pos: None,
            until: None,
            count: None,
        }
    }

    /// Parses an `every:` value: `day`, `weekday`, `week`, `2w`, `mon,thu`,
    /// `month`, `month@15`, `month@last`, `month@2tue`, `month@lastfri`,
    /// `year`, or a raw `FREQ=...` RRULE.
    pub fn from_shorthand(input: &str) -> Result<RecurRule, RecurError> {
        let err = |reason: &str| RecurError {
            input: input.to_owned(),
            reason: reason.to_owned(),
        };
        let s = input.trim().to_ascii_lowercase();
        if s.starts_with("freq=") {
            return input.parse();
        }
        let rule = match s.as_str() {
            "day" | "days" | "daily" => RecurRule::new(Freq::Daily),
            "weekday" | "weekdays" => RecurRule {
                by_day: vec![
                    Weekday::Monday,
                    Weekday::Tuesday,
                    Weekday::Wednesday,
                    Weekday::Thursday,
                    Weekday::Friday,
                ],
                ..RecurRule::new(Freq::Weekly)
            },
            "weekend" | "weekends" => RecurRule {
                by_day: vec![Weekday::Saturday, Weekday::Sunday],
                ..RecurRule::new(Freq::Weekly)
            },
            "week" | "weeks" | "weekly" => RecurRule::new(Freq::Weekly),
            "month" | "months" | "monthly" => RecurRule::new(Freq::Monthly),
            "year" | "years" | "yearly" | "annually" => RecurRule::new(Freq::Yearly),
            _ => {
                if let Some(day_spec) = s.strip_prefix("month@") {
                    let mut rule = RecurRule::new(Freq::Monthly);
                    parse_month_day_spec(day_spec, &mut rule)
                        .ok_or_else(|| err("try month@15, month@last or month@2tue"))?;
                    rule
                } else if let Some(days) = parse_weekday_list(&s) {
                    RecurRule {
                        by_day: days,
                        ..RecurRule::new(Freq::Weekly)
                    }
                } else {
                    parse_interval(&s).ok_or_else(|| {
                        err("try day, weekday, week, 2w, mon,thu, month@15 or year")
                    })?
                }
            }
        };
        Ok(rule)
    }

    /// Serialises to the stored RRULE form, e.g. `FREQ=WEEKLY;INTERVAL=2;BYDAY=MO`.
    pub fn to_rrule(&self) -> String {
        let mut parts = vec![format!(
            "FREQ={}",
            match self.freq {
                Freq::Daily => "DAILY",
                Freq::Weekly => "WEEKLY",
                Freq::Monthly => "MONTHLY",
                Freq::Yearly => "YEARLY",
            }
        )];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() {
            let days: Vec<_> = self.by_day.iter().map(|d| rrule_day(*d)).collect();
            parts.push(format!("BYDAY={}", days.join(",")));
        }
        if !self.by_month_day.is_empty() {
            let days: Vec<_> = self.by_month_day.iter().map(|d| d.to_string()).collect();
            parts.push(format!("BYMONTHDAY={}", days.join(",")));
        }
        if let Some(pos) = self.by_set_pos {
            parts.push(format!("BYSETPOS={pos}"));
        }
        if let Some(until) = self.until {
            parts.push(format!("UNTIL={}", until.strftime("%Y%m%d")));
        }
        if let Some(count) = self.count {
            parts.push(format!("COUNT={count}"));
        }
        parts.join(";")
    }

    /// Plain-English summary, e.g. "every 2 weeks on Mon, Thu".
    pub fn describe(&self) -> String {
        let unit = match self.freq {
            Freq::Daily => "day",
            Freq::Weekly => "week",
            Freq::Monthly => "month",
            Freq::Yearly => "year",
        };
        let base = match (self.freq, self.interval) {
            (Freq::Weekly, 1)
                if self.by_day.len() == 5
                    && !self.by_day.contains(&Weekday::Saturday)
                    && !self.by_day.contains(&Weekday::Sunday) =>
            {
                return self.with_limits("every weekday".to_owned());
            }
            (Freq::Daily, 1) => "daily".to_owned(),
            (Freq::Weekly, 1) => "weekly".to_owned(),
            (Freq::Monthly, 1) => "monthly".to_owned(),
            (Freq::Yearly, 1) => "yearly".to_owned(),
            (_, n) => format!("every {n} {unit}s"),
        };
        let detail = if !self.by_day.is_empty() {
            let days: Vec<_> = self.by_day.iter().map(|d| weekday_abbrev(*d)).collect();
            match self.by_set_pos {
                Some(pos) => format!(" on the {} {}", ordinal(pos), days.join(", ")),
                None => format!(" on {}", days.join(", ")),
            }
        } else if !self.by_month_day.is_empty() {
            let days: Vec<_> = self
                .by_month_day
                .iter()
                .map(|d| {
                    if *d == -1 {
                        "last day".to_owned()
                    } else {
                        format!("day {d}")
                    }
                })
                .collect();
            format!(" on {}", days.join(", "))
        } else {
            String::new()
        };
        self.with_limits(base + &detail)
    }

    fn with_limits(&self, mut text: String) -> String {
        if let Some(until) = self.until {
            text.push_str(&format!(" until {until}"));
        }
        if let Some(count) = self.count {
            text.push_str(&format!(" ({count} left)"));
        }
        text
    }

    /// Whether `day` is an occurrence of a series that started at `anchor`.
    pub fn occurs_on(&self, day: Date, anchor: Date) -> bool {
        if day < anchor || self.until.is_some_and(|until| day > until) {
            return false;
        }
        let interval = i64::from(self.interval.max(1));
        match self.freq {
            Freq::Daily => {
                days_between(anchor, day) % interval == 0
                    && (self.by_day.is_empty() || self.by_day.contains(&day.weekday()))
            }
            Freq::Weekly => {
                let weeks = days_between(monday_of(anchor), monday_of(day)) / 7;
                let days: &[Weekday] = if self.by_day.is_empty() {
                    &[anchor.weekday()]
                } else {
                    &self.by_day
                };
                weeks % interval == 0 && days.contains(&day.weekday())
            }
            Freq::Monthly => {
                let months = month_index(day) - month_index(anchor);
                if months % interval != 0 {
                    return false;
                }
                if !self.by_day.is_empty() {
                    if !self.by_day.contains(&day.weekday()) {
                        return false;
                    }
                    return match self.by_set_pos {
                        None => true,
                        Some(pos) if pos > 0 => (day.day() - 1) / 7 + 1 == pos,
                        Some(pos) => {
                            let from_end = (day.days_in_month() - day.day()) / 7 + 1;
                            from_end == -pos
                        }
                    };
                }
                let wanted: &[i8] = if self.by_month_day.is_empty() {
                    &[anchor.day()]
                } else {
                    &self.by_month_day
                };
                wanted
                    .iter()
                    .any(|md| resolve_month_day(*md, day.days_in_month()) == day.day())
            }
            Freq::Yearly => {
                let years = i64::from(day.year() - anchor.year());
                years % interval == 0
                    && day.month() == anchor.month()
                    && day.day() == anchor.day().min(day.days_in_month())
            }
        }
    }

    /// The first occurrence strictly after `after`.
    pub fn next_after(&self, after: Date, anchor: Date) -> Option<Date> {
        let period_days: i64 = match self.freq {
            Freq::Daily => 1,
            Freq::Weekly => 7,
            Freq::Monthly => 31,
            Freq::Yearly => 366,
        };
        let horizon = period_days * i64::from(self.interval.max(1)) * 2 + 400;
        let mut day = after.tomorrow().ok()?;
        for _ in 0..horizon {
            if self.until.is_some_and(|until| day > until) {
                return None;
            }
            if self.occurs_on(day, anchor) {
                return Some(day);
            }
            day = day.tomorrow().ok()?;
        }
        None
    }

    /// The rule to store on the next instance, or `None` once COUNT is used up.
    pub fn advanced(&self) -> Option<RecurRule> {
        match self.count {
            Some(n) if n <= 1 => None,
            Some(n) => Some(RecurRule {
                count: Some(n - 1),
                ..self.clone()
            }),
            None => Some(self.clone()),
        }
    }
}

/// Due date for the instance that replaces one completed `today`.
///
/// `Due` mode keeps the schedule: the first occurrence after the old due date
/// that is also after today, so missed occurrences collapse into one.
/// `Completion` mode counts from today.
pub fn next_due(
    rule: &RecurRule,
    mode: RecurMode,
    current_due: Option<Date>,
    today: Date,
) -> Option<Date> {
    match mode {
        RecurMode::Due => {
            let anchor = current_due.unwrap_or(today);
            let mut next = rule.next_after(anchor, anchor)?;
            while next <= today {
                next = rule.next_after(next, anchor)?;
            }
            Some(next)
        }
        RecurMode::Completion => rule.next_after(today, today),
    }
}

impl FromStr for RecurRule {
    type Err = RecurError;

    /// Parses the stored RRULE subset.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let err = |reason: String| RecurError {
            input: input.to_owned(),
            reason,
        };
        let mut rule: Option<RecurRule> = None;
        let mut rest: Vec<(String, String)> = Vec::new();
        for part in input.trim().split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| err(format!("expected KEY=VALUE, got `{part}`")))?;
            let key = key.trim().to_ascii_uppercase();
            let value = value.trim().to_ascii_uppercase();
            if key == "FREQ" {
                let freq = match value.as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    other => return Err(err(format!("unsupported FREQ `{other}`"))),
                };
                rule = Some(RecurRule::new(freq));
            } else {
                rest.push((key, value));
            }
        }
        let mut rule = rule.ok_or_else(|| err("missing FREQ".to_owned()))?;
        for (key, value) in rest {
            match key.as_str() {
                "INTERVAL" => {
                    rule.interval = value
                        .parse()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| err(format!("bad INTERVAL `{value}`")))?;
                }
                "BYDAY" => {
                    for item in value.split(',') {
                        let (pos, day) = item.split_at(item.len().saturating_sub(2));
                        let weekday = parse_rrule_day(day)
                            .ok_or_else(|| err(format!("bad BYDAY `{item}`")))?;
                        if !pos.is_empty() {
                            rule.by_set_pos = Some(
                                pos.parse()
                                    .map_err(|_| err(format!("bad BYDAY `{item}`")))?,
                            );
                        }
                        rule.by_day.push(weekday);
                    }
                }
                "BYMONTHDAY" => {
                    for item in value.split(',') {
                        let day: i8 = item
                            .parse()
                            .ok()
                            .filter(|d: &i8| *d != 0 && (-31..=31).contains(d))
                            .ok_or_else(|| err(format!("bad BYMONTHDAY `{item}`")))?;
                        rule.by_month_day.push(day);
                    }
                }
                "BYSETPOS" => {
                    rule.by_set_pos = Some(
                        value
                            .parse()
                            .map_err(|_| err(format!("bad BYSETPOS `{value}`")))?,
                    );
                }
                "UNTIL" => {
                    let digits: String =
                        value.chars().filter(char::is_ascii_digit).take(8).collect();
                    let parse = || -> Option<Date> {
                        Date::new(
                            digits.get(0..4)?.parse().ok()?,
                            digits.get(4..6)?.parse().ok()?,
                            digits.get(6..8)?.parse().ok()?,
                        )
                        .ok()
                    };
                    rule.until = Some(parse().ok_or_else(|| err(format!("bad UNTIL `{value}`")))?);
                }
                "COUNT" => {
                    rule.count = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|n| *n >= 1)
                            .ok_or_else(|| err(format!("bad COUNT `{value}`")))?,
                    );
                }
                other => return Err(err(format!("unsupported part `{other}`"))),
            }
        }
        Ok(rule)
    }
}

impl fmt::Display for RecurRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rrule())
    }
}

fn parse_interval(s: &str) -> Option<RecurRule> {
    let unit = s.chars().last()?;
    let n: u32 = s[..s.len() - 1].parse().ok().filter(|n| *n >= 1)?;
    let freq = match unit {
        'd' => Freq::Daily,
        'w' => Freq::Weekly,
        'm' => Freq::Monthly,
        'y' => Freq::Yearly,
        _ => return None,
    };
    Some(RecurRule {
        interval: n,
        ..RecurRule::new(freq)
    })
}

fn parse_weekday_list(s: &str) -> Option<Vec<Weekday>> {
    s.split(',').map(parse_weekday).collect()
}

/// `15`, `last`, `2tue`, `lastfri`, `-1fri`.
fn parse_month_day_spec(s: &str, rule: &mut RecurRule) -> Option<()> {
    if s == "last" {
        rule.by_month_day = vec![-1];
        return Some(());
    }
    if let Ok(day) = s.parse::<i8>() {
        if (1..=31).contains(&day) {
            rule.by_month_day = vec![day];
            return Some(());
        }
        return None;
    }
    let (pos, day) = if let Some(day) = s.strip_prefix("last") {
        (-1, day)
    } else {
        let split = s.find(|c: char| c.is_ascii_alphabetic())?;
        let pos: i8 = s[..split]
            .parse()
            .ok()
            .filter(|p| (-5..=5).contains(p) && *p != 0)?;
        (pos, &s[split..])
    };
    rule.by_day = vec![parse_weekday(day)?];
    rule.by_set_pos = Some(pos);
    Some(())
}

fn resolve_month_day(md: i8, days_in_month: i8) -> i8 {
    if md > 0 {
        md.min(days_in_month)
    } else {
        (days_in_month + md + 1).max(1)
    }
}

fn monday_of(date: Date) -> Date {
    date.checked_sub(i64::from(date.weekday().to_monday_zero_offset()).days())
        .expect("date in range")
}

fn month_index(date: Date) -> i64 {
    i64::from(date.year()) * 12 + i64::from(date.month())
}

fn rrule_day(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "MO",
        Weekday::Tuesday => "TU",
        Weekday::Wednesday => "WE",
        Weekday::Thursday => "TH",
        Weekday::Friday => "FR",
        Weekday::Saturday => "SA",
        Weekday::Sunday => "SU",
    }
}

fn parse_rrule_day(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

fn ordinal(pos: i8) -> String {
    match pos {
        -1 => "last".to_owned(),
        1 => "1st".to_owned(),
        2 => "2nd".to_owned(),
        3 => "3rd".to_owned(),
        n => format!("{n}th"),
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn rule(s: &str) -> RecurRule {
        RecurRule::from_shorthand(s).unwrap()
    }

    #[test]
    fn shorthands_compile_to_rrule() {
        assert_eq!(rule("day").to_rrule(), "FREQ=DAILY");
        assert_eq!(rule("3d").to_rrule(), "FREQ=DAILY;INTERVAL=3");
        assert_eq!(
            rule("weekday").to_rrule(),
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"
        );
        assert_eq!(rule("2w").to_rrule(), "FREQ=WEEKLY;INTERVAL=2");
        assert_eq!(rule("mon,thu").to_rrule(), "FREQ=WEEKLY;BYDAY=MO,TH");
        assert_eq!(rule("month@15").to_rrule(), "FREQ=MONTHLY;BYMONTHDAY=15");
        assert_eq!(rule("month@last").to_rrule(), "FREQ=MONTHLY;BYMONTHDAY=-1");
        assert_eq!(
            rule("month@2tue").to_rrule(),
            "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"
        );
        assert_eq!(
            rule("month@lastfri").to_rrule(),
            "FREQ=MONTHLY;BYDAY=FR;BYSETPOS=-1"
        );
        assert_eq!(rule("year").to_rrule(), "FREQ=YEARLY");
        assert!(RecurRule::from_shorthand("fortnightly").is_err());
        assert!(RecurRule::from_shorthand("month@32").is_err());
    }

    #[test]
    fn rrule_round_trips() {
        for s in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR",
            "FREQ=MONTHLY;BYMONTHDAY=-1",
            "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2",
            "FREQ=YEARLY;UNTIL=20301231;COUNT=3",
        ] {
            assert_eq!(s.parse::<RecurRule>().unwrap().to_rrule(), s);
        }
        assert_eq!(
            "FREQ=MONTHLY;BYDAY=2TU"
                .parse::<RecurRule>()
                .unwrap()
                .to_rrule(),
            "FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2"
        );
        assert!("FREQ=HOURLY".parse::<RecurRule>().is_err());
        assert!("INTERVAL=2".parse::<RecurRule>().is_err());
    }

    #[test]
    fn weekly_keeps_cadence_and_interval() {
        let anchor = date(2026, 10, 15); // Thu
        assert_eq!(
            rule("week").next_after(anchor, anchor),
            Some(date(2026, 10, 22))
        );
        assert_eq!(
            rule("2w").next_after(anchor, anchor),
            Some(date(2026, 10, 29))
        );
        assert_eq!(
            rule("mon,thu").next_after(anchor, anchor),
            Some(date(2026, 10, 19))
        );
        assert_eq!(
            rule("weekday").next_after(date(2026, 10, 16), anchor),
            Some(date(2026, 10, 19))
        );
    }

    #[test]
    fn monthly_variants() {
        let anchor = date(2026, 1, 31);
        // Clamped to the last day in short months.
        assert_eq!(
            rule("month").next_after(anchor, anchor),
            Some(date(2026, 2, 28))
        );
        let last = rule("month@last");
        assert_eq!(
            last.next_after(date(2026, 2, 1), anchor),
            Some(date(2026, 2, 28))
        );
        let second_tue = rule("month@2tue");
        assert_eq!(
            second_tue.next_after(date(2026, 10, 1), date(2026, 10, 1)),
            Some(date(2026, 10, 13))
        );
        let last_fri = rule("month@lastfri");
        assert_eq!(
            last_fri.next_after(date(2026, 10, 1), date(2026, 10, 1)),
            Some(date(2026, 10, 30))
        );
    }

    #[test]
    fn yearly_handles_leap_day() {
        let anchor = date(2028, 2, 29);
        assert_eq!(
            rule("year").next_after(anchor, anchor),
            Some(date(2029, 2, 28))
        );
    }

    #[test]
    fn until_stops_series() {
        let mut r = rule("week");
        r.until = Some(date(2026, 10, 20));
        let anchor = date(2026, 10, 15);
        assert_eq!(r.next_after(anchor, anchor), None);
    }

    #[test]
    fn count_runs_out() {
        let mut r = rule("day");
        r.count = Some(2);
        let next = r.advanced().unwrap();
        assert_eq!(next.count, Some(1));
        assert_eq!(next.advanced(), None);
    }

    #[test]
    fn missed_occurrences_collapse_into_one() {
        // Weekly review due Thu Oct 1; completed late on Tue Oct 20.
        let next = next_due(
            &rule("week"),
            RecurMode::Due,
            Some(date(2026, 10, 1)),
            date(2026, 10, 20),
        );
        assert_eq!(next, Some(date(2026, 10, 22)));
        // On time: next week.
        let next = next_due(
            &rule("week"),
            RecurMode::Due,
            Some(date(2026, 10, 15)),
            date(2026, 10, 15),
        );
        assert_eq!(next, Some(date(2026, 10, 22)));
        // Daily done late: tomorrow, not today.
        let next = next_due(
            &rule("day"),
            RecurMode::Due,
            Some(date(2026, 10, 10)),
            date(2026, 10, 15),
        );
        assert_eq!(next, Some(date(2026, 10, 16)));
    }

    #[test]
    fn completion_mode_counts_from_today() {
        let next = next_due(
            &rule("3d"),
            RecurMode::Completion,
            Some(date(2026, 10, 1)),
            date(2026, 10, 15),
        );
        assert_eq!(next, Some(date(2026, 10, 18)));
    }

    #[test]
    fn descriptions() {
        assert_eq!(rule("day").describe(), "daily");
        assert_eq!(rule("weekday").describe(), "every weekday");
        assert_eq!(rule("2w").describe(), "every 2 weeks");
        assert_eq!(rule("mon,thu").describe(), "weekly on Mon, Thu");
        assert_eq!(rule("month@2tue").describe(), "monthly on the 2nd Tue");
        assert_eq!(rule("month@last").describe(), "monthly on last day");
    }
}
