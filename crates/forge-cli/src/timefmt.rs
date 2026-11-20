//! Wall-clock formatting and parsing shared by the CLI surfaces. Every format string here is
//! strftime-style and is part of what Forge writes or prints, so the tests pin the exact output.

use jiff::{tz::TimeZone, Timestamp};

pub(crate) fn utc(secs: i64, fmt: &str) -> Option<String> {
    let at = Timestamp::from_second(secs).ok()?;
    Some(at.to_zoned(TimeZone::UTC).strftime(fmt).to_string())
}

pub(crate) fn utc_millis(ms: i64, fmt: &str) -> Option<String> {
    let at = Timestamp::from_millisecond(ms).ok()?;
    Some(at.to_zoned(TimeZone::UTC).strftime(fmt).to_string())
}

pub(crate) fn local(secs: i64, fmt: &str) -> Option<String> {
    let at = Timestamp::from_second(secs).ok()?;
    Some(at.to_zoned(TimeZone::system()).strftime(fmt).to_string())
}

pub(crate) fn local_now(fmt: &str) -> String {
    jiff::Zoned::now().strftime(fmt).to_string()
}

/// RFC 3339 in UTC with a `+00:00` suffix and as many fraction digits as the instant needs
/// (none, 3, 6 or 9), the shape the audit log has always carried.
pub(crate) fn rfc3339_auto(secs: i64, nanos: u32) -> String {
    let total = i128::from(secs) * 1_000_000_000 + i128::from(nanos);
    let at = Timestamp::from_nanosecond(total).unwrap_or_default();
    let sub = total.rem_euclid(1_000_000_000);
    let fraction = if sub == 0 {
        String::new()
    } else if sub % 1_000_000 == 0 {
        format!(".{:03}", sub / 1_000_000)
    } else if sub % 1_000 == 0 {
        format!(".{:06}", sub / 1_000)
    } else {
        format!(".{sub:09}")
    };
    let seconds = at.to_zoned(TimeZone::UTC).strftime("%Y-%m-%dT%H:%M:%S");
    format!("{seconds}{fraction}+00:00")
}

pub(crate) fn now_rfc3339_auto() -> String {
    let now = Timestamp::now().as_nanosecond();
    let secs = i64::try_from(now.div_euclid(1_000_000_000)).unwrap_or_default();
    let nanos = u32::try_from(now.rem_euclid(1_000_000_000)).unwrap_or_default();
    rfc3339_auto(secs, nanos)
}

/// Epoch seconds of an RFC 3339 timestamp, rounded down like every other epoch in the store.
pub(crate) fn parse_rfc3339(text: &str) -> Option<i64> {
    let at: Timestamp = text.parse().ok()?;
    Some(at.as_second() - i64::from(at.subsec_nanosecond() < 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formats_are_exact() {
        assert_eq!(
            utc(1_700_000_000, "%Y-%m-%d %H:%M UTC").as_deref(),
            Some("2023-11-14 22:13 UTC")
        );
        assert_eq!(utc(0, "%Y-%m-%d").as_deref(), Some("1970-01-01"));
        assert_eq!(
            utc(-86_400, "%Y-%m-%d %H:%M UTC").as_deref(),
            Some("1969-12-31 00:00 UTC")
        );
        assert_eq!(utc(951_782_400, "%Y-%m-%d").as_deref(), Some("2000-02-29"));
        assert_eq!(utc(i64::MAX, "%Y-%m-%d"), None);
    }

    #[test]
    fn utc_millis_keep_three_fraction_digits_and_the_z() {
        let fmt = "%Y-%m-%dT%H:%M:%S%.3fZ";
        assert_eq!(
            utc_millis(0, fmt).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
        assert_eq!(
            utc_millis(1_700_000_000_123, fmt).as_deref(),
            Some("2023-11-14T22:13:20.123Z")
        );
        assert_eq!(
            utc_millis(1_700_000_000_007, fmt).as_deref(),
            Some("2023-11-14T22:13:20.007Z")
        );
        assert_eq!(utc_millis(i64::MAX, fmt), None);
    }

    fn shape(text: &str, pattern: &str) -> bool {
        text.len() == pattern.len()
            && text.chars().zip(pattern.chars()).all(|(t, p)| match p {
                '9' => t.is_ascii_digit(),
                'a' => t.is_ascii_alphabetic(),
                p => t == p,
            })
    }

    #[test]
    fn local_formats_have_the_printed_shapes() {
        for secs in [0i64, 1_700_000_000, 1_786_314_600, 1_794_267_000] {
            let t = local(secs, "%m-%d %H:%M").unwrap();
            assert!(shape(&t, "99-99 99:99"), "{t}");
            let t = local(secs, "%Y-%m-%d %H:%M:%S").unwrap();
            assert!(shape(&t, "9999-99-99 99:99:99"), "{t}");
            let t = local(secs, "%a %H:%M").unwrap();
            assert!(shape(&t, "aaa 99:99"), "{t}");
            let t = local(secs, "%H:%M").unwrap();
            assert!(shape(&t, "99:99"), "{t}");
        }
        assert_eq!(local(i64::MAX, "%H:%M"), None);
        let stamp = local_now("%Y%m%d-%H%M%S");
        assert!(shape(&stamp, "99999999-999999"), "{stamp}");
    }

    #[test]
    fn local_weekday_names_are_english_abbreviations() {
        let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        for step in 0..7 {
            let t = local(1_700_000_000 + step * 86_400, "%a").unwrap();
            assert!(days.contains(&t.as_str()), "{t}");
        }
    }

    #[test]
    fn auto_rfc3339_uses_plus_zero_and_the_shortest_fraction_that_fits() {
        assert_eq!(rfc3339_auto(0, 0), "1970-01-01T00:00:00+00:00");
        assert_eq!(rfc3339_auto(1_700_000_000, 0), "2023-11-14T22:13:20+00:00");
        assert_eq!(
            rfc3339_auto(1_700_000_000, 120_000_000),
            "2023-11-14T22:13:20.120+00:00"
        );
        assert_eq!(
            rfc3339_auto(1_700_000_000, 123_456_000),
            "2023-11-14T22:13:20.123456+00:00"
        );
        assert_eq!(
            rfc3339_auto(1_700_000_000, 123_456_789),
            "2023-11-14T22:13:20.123456789+00:00"
        );
        assert_eq!(
            rfc3339_auto(1_700_000_000, 7_000),
            "2023-11-14T22:13:20.000007+00:00"
        );
        assert_eq!(
            rfc3339_auto(-1, 500_000_000),
            "1969-12-31T23:59:59.500+00:00"
        );
        let live = now_rfc3339_auto();
        assert!(live.ends_with("+00:00") && live.starts_with("20"), "{live}");
    }

    #[test]
    fn rfc3339_parses_to_epoch_seconds() {
        assert_eq!(
            parse_rfc3339("2026-09-02T00:30:48.917Z"),
            Some(1_788_309_048)
        );
        assert_eq!(
            parse_rfc3339("2026-09-14T22:21:14.589596Z"),
            Some(1_789_424_474)
        );
        assert_eq!(
            parse_rfc3339("2023-11-14T23:13:20+01:00"),
            Some(1_700_000_000)
        );
        assert_eq!(
            parse_rfc3339("2023-11-14T22:13:20-00:00"),
            Some(1_700_000_000)
        );
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59Z"), Some(-1));
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59.500Z"), Some(-1));
        assert_eq!(parse_rfc3339("2023-11-14"), None);
        assert_eq!(parse_rfc3339("not a time"), None);
        assert_eq!(parse_rfc3339(""), None);
    }
}
