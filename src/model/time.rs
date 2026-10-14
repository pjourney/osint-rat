//! Robust parsing of untrusted timestamps. All values are stored as UTC.

use chrono::{DateTime, NaiveDate, NaiveDateTime, SubsecRound, TimeZone, Utc};

use super::common::Timestamp;

/// Parses an untrusted timestamp into UTC milliseconds.
///
/// Accepts RFC 3339 (any offset → UTC), date-only (`YYYY-MM-DD` as midnight UTC),
/// a few naive datetime layouts treated as UTC, and unix seconds/milliseconds.
/// Empty, `NaN`, out-of-range, and malformed values return `None`. Never panics.
pub fn parse_timestamp_untrusted(raw: &str) -> Option<Timestamp> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_non_instant(trimmed) {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.with_timezone(&Utc).trunc_subsecs(3));
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return date
            .and_hms_opt(0, 0, 0)
            .map(|naive| Utc.from_utc_datetime(&naive).trunc_subsecs(3));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(Utc.from_utc_datetime(&naive).trunc_subsecs(3));
        }
    }
    parse_unix_timestamp(trimmed)
}

fn is_non_instant(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "nan"
            | "+nan"
            | "-nan"
            | "inf"
            | "+inf"
            | "-inf"
            | "infinity"
            | "+infinity"
            | "-infinity"
            | "null"
            | "none"
            | "undefined"
    )
}

fn parse_unix_timestamp(raw: &str) -> Option<Timestamp> {
    if !raw
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    let value: i64 = raw.parse().ok()?;
    if value.abs() >= 1_000_000_000_000 {
        DateTime::from_timestamp_millis(value).map(|stamp| stamp.trunc_subsecs(3))
    } else {
        DateTime::from_timestamp(value, 0).map(|stamp| stamp.trunc_subsecs(3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn rfc3339_offsets_become_utc() {
        let actual = parse_timestamp_untrusted("2024-06-01T12:00:00-04:00").expect("parsed");
        assert_eq!(actual, Utc.with_ymd_and_hms(2024, 6, 1, 16, 0, 0).unwrap());
        assert_eq!(actual.timezone(), Utc);
    }

    #[test]
    fn date_only_is_midnight_utc() {
        let actual = parse_timestamp_untrusted("2011-08-11").expect("parsed");
        assert_eq!(actual, Utc.with_ymd_and_hms(2011, 8, 11, 0, 0, 0).unwrap());
        assert_eq!(actual.hour(), 0);
    }

    #[test]
    fn malformed_nan_and_empty_do_not_panic() {
        for input in [
            "",
            "   ",
            "NaN",
            "nan",
            "Infinity",
            "not-a-date",
            "2024-13-40",
            "99:99:99",
            "null",
        ] {
            assert!(
                parse_timestamp_untrusted(input).is_none(),
                "input {input:?} must be refused"
            );
        }
    }

    #[test]
    fn unix_seconds_round_trip_utc() {
        let actual = parse_timestamp_untrusted("1710000000").expect("unix");
        assert_eq!(actual.timezone(), Utc);
        assert_eq!(actual.timestamp(), 1_710_000_000);
    }
}
