//! Minimal ISO 8601 / RFC 3339 dates (UTC), enough for KeePassXC and xPass CSV files.
//! No calendar dependency: civil-date arithmetic from Howard Hinnant's algorithms.

/// Parses `YYYY-MM-DD`, optionally followed by `T` or a space, `HH:MM[:SS[.fff]]` and a
/// zone (`Z`, `±HH:MM`, `±HHMM`; none = UTC). Returns Unix milliseconds.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let s = s.trim().as_bytes();
    if s.len() < 10 || s[4] != b'-' || s[7] != b'-' {
        return None;
    }
    let year = digits(&s[0..4])?;
    let month = digits(&s[5..7])?;
    let day = digits(&s[8..10])?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut ms = days_from_civil(year, month, day) * 86_400_000;

    let mut rest = &s[10..];
    if rest.is_empty() {
        return Some(ms);
    }
    if !matches!(rest[0], b'T' | b't' | b' ') || rest.len() < 6 || rest[3] != b':' {
        return None;
    }
    let hour = digits(&rest[1..3])?;
    let minute = digits(&rest[4..6])?;
    rest = &rest[6..];
    let mut second = 0;
    let mut millis = 0;
    if rest.first() == Some(&b':') {
        second = digits(rest.get(1..3)?)?;
        rest = &rest[3..];
        if rest.first() == Some(&b'.') {
            let end = rest[1..]
                .iter()
                .position(|b| !b.is_ascii_digit())
                .map_or(rest.len(), |p| p + 1);
            let frac = &rest[1..end];
            if frac.is_empty() {
                return None;
            }
            // First three digits are milliseconds; pad shorter fractions.
            let mut m = 0;
            for i in 0..3 {
                m = m * 10 + frac.get(i).map_or(0, |d| i64::from(d - b'0'));
            }
            millis = m;
            rest = &rest[end..];
        }
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // A leap second is folded into the last second of the minute.
    ms += ((hour * 60 + minute) * 60 + second.min(59)) * 1000 + millis;

    let offset_min = match rest {
        [] | [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] | [sign @ (b'+' | b'-'), h1, h2, m1, m2] => {
            let h = digits(&[*h1, *h2])?;
            let m = digits(&[*m1, *m2])?;
            if h > 23 || m > 59 {
                return None;
            }
            let total = h * 60 + m;
            if *sign == b'+' {
                total
            } else {
                -total
            }
        }
        _ => return None,
    };
    Some(ms - offset_min * 60_000)
}

/// Formats Unix milliseconds as `YYYY-MM-DDTHH:MM:SS.sssZ`.
pub fn format_iso8601(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (secs, millis) = (in_day / 1000, in_day % 1000);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn digits(b: &[u8]) -> Option<i64> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(b.iter().fold(0, |acc, d| acc * 10 + i64::from(d - b'0')))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_iso8601("1970-01-01"), Some(0));
        assert_eq!(parse_iso8601("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_iso8601("2024-01-15T08:30:00Z"), Some(1_705_307_400_000));
        assert_eq!(parse_iso8601("2024-01-15 08:30:00"), Some(1_705_307_400_000));
        assert_eq!(parse_iso8601("2024-01-15T08:30"), Some(1_705_307_400_000));
        assert_eq!(parse_iso8601("2024-01-15T10:30:00+02:00"), Some(1_705_307_400_000));
        assert_eq!(parse_iso8601("2024-01-15T03:30:00-0500"), Some(1_705_307_400_000));
        assert_eq!(parse_iso8601("2024-01-15T08:30:00.5Z"), Some(1_705_307_400_500));
        assert_eq!(parse_iso8601("2024-01-15T08:30:00.123456Z"), Some(1_705_307_400_123));
        assert_eq!(parse_iso8601(" 2024-02-29 "), Some(1_709_164_800_000));
    }

    #[test]
    fn rejects_garbage() {
        for s in [
            "",
            "not-a-date",
            "2024-13-01",
            "2023-02-29",
            "2024-01-32",
            "2024-01-15T24:00:00Z",
            "2024-01-15T08:30:00.Z",
            "2024-01-15T08:30:00+25:00",
            "2024-01-15X08:30",
            "2024/01/15",
        ] {
            assert_eq!(parse_iso8601(s), None, "{s}");
        }
    }

    #[test]
    fn format_round_trips() {
        for ms in [0, 1_705_307_400_123, 951_782_400_000, 4_102_444_799_999, -86_400_000] {
            assert_eq!(parse_iso8601(&format_iso8601(ms)), Some(ms), "{ms}");
        }
        assert_eq!(format_iso8601(1_705_307_400_123), "2024-01-15T08:30:00.123Z");
    }
}
