//! Date phrases used by quick-add (`due:fri`), the filter language
//! (`due:<=today`) and the TUI date prompt.

use jiff::ToSpan;
use jiff::civil::{Date, Time, Weekday};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot understand date `{input}`: {reason}")]
pub struct DateError {
    pub input: String,
    pub reason: &'static str,
}

/// A calendar date with an optional time of day (floating local time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateSpec {
    pub date: Date,
    pub time: Option<Time>,
}

/// Parses phrases such as `today`, `tom`, `fri`, `+3d`, `-1w`, `eom`,
/// `2026-12-24`, `12-24`, optionally followed by `@17:00`, `@5pm`.
pub fn parse_date(input: &str, today: Date) -> Result<DateSpec, DateError> {
    let err = |reason| DateError {
        input: input.to_owned(),
        reason,
    };
    let lower = input.trim().to_ascii_lowercase();
    let (date_part, time_part) = match lower.split_once('@') {
        Some((d, t)) => (d, Some(t)),
        None => (lower.as_str(), None),
    };
    let date = if date_part.is_empty() {
        if time_part.is_none() {
            return Err(err("empty date"));
        }
        today
    } else {
        parse_date_only(date_part, today).ok_or_else(|| err("unknown date phrase"))?
    };
    let time = match time_part {
        Some(t) => Some(parse_time(t).ok_or_else(|| err("unknown time, try 17:00 or 5pm"))?),
        None => None,
    };
    Ok(DateSpec { date, time })
}

fn parse_date_only(s: &str, today: Date) -> Option<Date> {
    match s {
        "today" | "tod" | "now" => return Some(today),
        "tomorrow" | "tom" | "tmr" => return today.tomorrow().ok(),
        "yesterday" | "yest" => return today.yesterday().ok(),
        "sow" => return Some(start_of_week(today)),
        "eow" => return Some(end_of_week(today)),
        "eom" => return Some(today.last_of_month()),
        "eoy" => return Date::new(today.year(), 12, 31).ok(),
        "nw" | "next-week" | "nextweek" => {
            return end_of_week(today).tomorrow().ok();
        }
        "nm" | "next-month" | "nextmonth" => {
            return today.last_of_month().tomorrow().ok();
        }
        _ => {}
    }
    if let Some(weekday) = parse_weekday(s) {
        return Some(next_weekday_on_or_after(today, weekday));
    }
    if let Some(date) = parse_offset(s, today) {
        return Some(date);
    }
    parse_absolute(s, today)
}

/// `+3d`, `3d`, `-1w`, `+2m`, `+1y`.
fn parse_offset(s: &str, today: Date) -> Option<Date> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1i64, &s[1..]),
        b'-' => (-1i64, &s[1..]),
        b'0'..=b'9' => (1i64, s),
        _ => return None,
    };
    let unit = rest.chars().last()?;
    let amount: i64 = rest[..rest.len() - unit.len_utf8()].parse().ok()?;
    let n = sign * amount;
    let span = match unit {
        'd' => n.days(),
        'w' => n.weeks(),
        'm' => n.months(),
        'y' => n.years(),
        _ => return None,
    };
    today.checked_add(span).ok()
}

/// `2026-12-24` or `12-24` (this year, or next year if already past).
fn parse_absolute(s: &str, today: Date) -> Option<Date> {
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => {
            Date::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?).ok()
        }
        [m, d] => {
            let (m, d): (i8, i8) = (m.parse().ok()?, d.parse().ok()?);
            let this_year = Date::new(today.year(), m, d).ok()?;
            if this_year >= today {
                Some(this_year)
            } else {
                Date::new(today.year() + 1, m, d).ok()
            }
        }
        _ => None,
    }
}

/// `17:00`, `9:30`, `17`, `5pm`, `5:30pm`, `12am`.
pub fn parse_time(s: &str) -> Option<Time> {
    let s = s.trim().to_ascii_lowercase();
    let (body, meridiem) = if let Some(b) = s.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = s.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (s.as_str(), None)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<i8>().ok()?, m.parse::<i8>().ok()?),
        None => (body.parse::<i8>().ok()?, 0),
    };
    let h = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            match (h, pm) {
                (12, false) => 0,
                (12, true) => 12,
                (h, true) => h + 12,
                (h, false) => h,
            }
        }
        None => h,
    };
    Time::new(h, m, 0, 0).ok()
}

pub fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    })
}

pub fn weekday_abbrev(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

/// The next date with `weekday`, counting today.
pub fn next_weekday_on_or_after(today: Date, weekday: Weekday) -> Date {
    let ahead =
        (weekday.to_monday_zero_offset() - today.weekday().to_monday_zero_offset()).rem_euclid(7);
    today
        .checked_add(i64::from(ahead).days())
        .expect("date in range")
}

/// Monday of the week containing `date`.
pub fn start_of_week(date: Date) -> Date {
    date.checked_sub(i64::from(date.weekday().to_monday_zero_offset()).days())
        .expect("date in range")
}

/// Sunday of the (Monday-based) week containing `date`.
pub fn end_of_week(date: Date) -> Date {
    next_weekday_on_or_after(date, Weekday::Sunday)
}

pub fn format_time(time: Time) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Short human label relative to `today`: `today`, `tomorrow`, `yesterday`,
/// a weekday within the next 6 days, `Nov 30`, or `Nov 30 2027` in another year.
pub fn relative_label(date: Date, today: Date) -> String {
    let days = days_between(today, date);
    match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        -1 => "yesterday".to_owned(),
        2..=6 => weekday_abbrev(date.weekday()).to_owned(),
        _ => {
            let month = MONTHS[(date.month() - 1) as usize];
            if date.year() == today.year() {
                format!("{month} {}", date.day())
            } else {
                format!("{month} {} {}", date.day(), date.year())
            }
        }
    }
}

/// Whole days from `from` to `to` (negative when `to` is earlier).
pub fn days_between(from: Date, to: Date) -> i64 {
    from.until(to)
        .map(|span| i64::from(span.get_days()))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    // Thursday.
    const TODAY: Date = date(2026, 10, 15);

    fn d(s: &str) -> Date {
        parse_date(s, TODAY).unwrap().date
    }

    #[test]
    fn keywords() {
        assert_eq!(d("today"), TODAY);
        assert_eq!(d("tom"), date(2026, 10, 16));
        assert_eq!(d("yesterday"), date(2026, 10, 14));
        assert_eq!(d("eow"), date(2026, 10, 18));
        assert_eq!(d("eom"), date(2026, 10, 31));
        assert_eq!(d("nw"), date(2026, 10, 19));
        assert_eq!(d("next-month"), date(2026, 11, 1));
    }

    #[test]
    fn weekdays_count_today() {
        assert_eq!(d("thu"), TODAY);
        assert_eq!(d("fri"), date(2026, 10, 16));
        assert_eq!(d("Monday"), date(2026, 10, 19));
    }

    #[test]
    fn offsets() {
        assert_eq!(d("+3d"), date(2026, 10, 18));
        assert_eq!(d("3d"), date(2026, 10, 18));
        assert_eq!(d("-1w"), date(2026, 10, 8));
        assert_eq!(d("+1m"), date(2026, 11, 15));
        assert_eq!(d("+1y"), date(2027, 10, 15));
    }

    #[test]
    fn absolute_dates() {
        assert_eq!(d("2026-12-24"), date(2026, 12, 24));
        assert_eq!(d("12-24"), date(2026, 12, 24));
        assert_eq!(d("01-02"), date(2027, 1, 2));
    }

    #[test]
    fn times() {
        let spec = parse_date("tom@17:30", TODAY).unwrap();
        assert_eq!(spec.time, Some(Time::new(17, 30, 0, 0).unwrap()));
        assert_eq!(parse_time("5pm"), Time::new(17, 0, 0, 0).ok());
        assert_eq!(parse_time("12am"), Time::new(0, 0, 0, 0).ok());
        assert_eq!(parse_time("9"), Time::new(9, 0, 0, 0).ok());
        let only_time = parse_date("@9am", TODAY).unwrap();
        assert_eq!(only_time.date, TODAY);
    }

    #[test]
    fn rejects_nonsense() {
        assert!(parse_date("someday", TODAY).is_err());
        assert!(parse_date("2026-13-01", TODAY).is_err());
        assert!(parse_date("tom@25:00", TODAY).is_err());
        assert!(parse_date("", TODAY).is_err());
    }

    #[test]
    fn labels() {
        assert_eq!(relative_label(TODAY, TODAY), "today");
        assert_eq!(relative_label(date(2026, 10, 16), TODAY), "tomorrow");
        assert_eq!(relative_label(date(2026, 10, 19), TODAY), "Mon");
        assert_eq!(relative_label(date(2026, 11, 30), TODAY), "Nov 30");
        assert_eq!(relative_label(date(2027, 1, 2), TODAY), "Jan 2 2027");
    }
}
