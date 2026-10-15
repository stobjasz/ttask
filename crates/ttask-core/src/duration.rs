//! Estimates such as `~30m`, `~2h`, `~1h30m`, `~90` (minutes).

pub fn parse_minutes(input: &str) -> Option<u32> {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Ok(minutes) = s.parse::<u32>() {
        return Some(minutes);
    }
    let mut total: u32 = 0;
    let mut number = String::new();
    let mut saw_unit = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: u32 = number.parse().ok()?;
        number.clear();
        total = total.checked_add(match c {
            'h' => n.checked_mul(60)?,
            'm' => n,
            'd' => n.checked_mul(8 * 60)?,
            _ => return None,
        })?;
        saw_unit = true;
    }
    (number.is_empty() && saw_unit).then_some(total)
}

pub fn format_minutes(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(parse_minutes("30m"), Some(30));
        assert_eq!(parse_minutes("2h"), Some(120));
        assert_eq!(parse_minutes("1h30m"), Some(90));
        assert_eq!(parse_minutes("90"), Some(90));
        assert_eq!(parse_minutes("1d"), Some(480));
        assert_eq!(parse_minutes("h"), None);
        assert_eq!(parse_minutes("2x"), None);
        assert_eq!(parse_minutes("1h3"), None);
    }

    #[test]
    fn formats() {
        assert_eq!(format_minutes(45), "45m");
        assert_eq!(format_minutes(120), "2h");
        assert_eq!(format_minutes(90), "1h30m");
    }
}
