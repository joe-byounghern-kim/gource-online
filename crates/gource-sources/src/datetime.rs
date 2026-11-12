// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Date strings in the timestamp column of a custom log
//! (`SDLAppSettings::parseDateTime`, `core/settings.cpp`).
//!
//! The C++ accepts exactly the strings matching
//!
//! ```text
//! ^(\d{4})-(\d{2})-(\d{2})
//!  (?:[T ](\d{1,2}):(\d{2})(?::(\d{2}(?:\.\d+)?))?)?
//!  (Z| ?([+-])(\d{1,2})(?::?(\d{2}))?)?$
//! ```
//!
//! and turns the captured fields into a Unix timestamp with `mktime`. This
//! module reproduces the match (including PCRE's backtracking order for the
//! zone offset, so `+530` is 5:30 and `+123` is 1:23) and the `mktime`
//! arithmetic:
//!
//! - fields are not range checked: month 13 is January of the next year, day
//!   45 rolls over into the following month, hour 25 and minute 61 carry,
//!   exactly like glibc's normalising `mktime`;
//! - fractional seconds are discarded;
//! - `Z` and numeric offsets are UTC based (`mktime_utc`);
//! - a string without a zone is *local time* in C++. A browser worker or a
//!   headless cloud run has no portable "local time zone" here, so the caller
//!   passes the offset to use (seconds east of UTC, `0` for UTC).

/// Parse `s` the way `SDLAppSettings::parseDateTime` does. `local_utc_offset`
/// is applied to strings that carry no zone designator (seconds east of UTC:
/// `+32400` for Seoul, `-18000` for New York in winter).
pub(crate) fn parse_date_time(s: &[u8], local_utc_offset: i64) -> Option<i64> {
    let m = match_date_time(s)?;

    let days = days_from_civil_normalised(m.year, m.month, m.day);
    let utc = days * 86_400 + m.hour * 3_600 + m.minute * 60 + m.second;

    Some(match m.zone {
        Zone::Local => utc - local_utc_offset,
        Zone::Utc => utc,
        Zone::Offset {
            negative,
            hour,
            minute,
        } => {
            let offset = hour * 3_600 + minute * 60;
            utc - if negative { -offset } else { offset }
        }
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Zone {
    /// No designator: `mktime` in the local time zone.
    Local,
    /// `Z`.
    Utc,
    /// `[+-]h[h][[:]mm]`.
    Offset {
        negative: bool,
        hour: i64,
        minute: i64,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    zone: Zone,
}

fn digits(s: &[u8]) -> Option<i64> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(s.iter().fold(0i64, |v, &b| v * 10 + i64::from(b - b'0')))
}

fn leading_digits(s: &[u8]) -> usize {
    s.iter().take_while(|b| b.is_ascii_digit()).count()
}

/// The optional time group `[T ](\d{1,2}):(\d{2})(?::(\d{2}(?:\.\d+)?))?`.
/// Returns `(hour, minute, second, bytes consumed)`.
fn match_time(s: &[u8]) -> Option<(i64, i64, i64, usize)> {
    if !matches!(s.first(), Some(b'T' | b' ')) {
        return None;
    }
    let rest = &s[1..];

    // `\d{1,2}` followed by ':'. Giving a digit back cannot help: the next
    // byte would be a digit, not a colon.
    let hour_len = leading_digits(rest);
    if !(1..=2).contains(&hour_len) || rest.get(hour_len) != Some(&b':') {
        return None;
    }
    let hour = digits(&rest[..hour_len])?;
    let mut at = 1 + hour_len + 1;

    let minute = digits(s.get(at..at + 2)?)?;
    at += 2;

    let mut second = 0;
    // `(?::(\d{2}(?:\.\d+)?))?`, greedy. Backtracking out of it never leads
    // to a match, since ':' and '.' cannot start the zone or end the string.
    if s.get(at) == Some(&b':')
        && let Some(sec) = s.get(at + 1..at + 3).and_then(digits)
    {
        second = sec;
        at += 3;
        if s.get(at) == Some(&b'.') {
            let frac = leading_digits(&s[at + 1..]);
            if frac > 0 {
                at += 1 + frac;
            }
        }
    }

    Some((hour, minute, second, at))
}

/// The optional zone group followed by the end of the string:
/// `(Z| ?([+-])(\d{1,2})(?::?(\d{2}))?)?$`.
fn match_zone_and_end(s: &[u8]) -> Option<Zone> {
    if s.is_empty() {
        return Some(Zone::Local);
    }
    if s == b"Z" {
        return Some(Zone::Utc);
    }

    let s = s.strip_prefix(b" ").unwrap_or(s);
    let (negative, s) = match s.split_first()? {
        (b'+', rest) => (false, rest),
        (b'-', rest) => (true, rest),
        _ => return None,
    };

    // `(\d{1,2})(?::?(\d{2}))?` then `$`, in PCRE's priority order: two hour
    // digits before one; a minutes part (with a colon, then without) before
    // none.
    let available = leading_digits(s).min(2);
    for hour_len in (1..=available).rev() {
        let hour = digits(&s[..hour_len])?;
        let after = &s[hour_len..];

        let minutes = if let Some(m) = after.strip_prefix(b":") {
            Some(m)
        } else if !after.is_empty() {
            Some(after)
        } else {
            None
        };

        match minutes {
            // minutes group present: exactly two digits then the end
            Some(m) => {
                if m.len() == 2
                    && let Some(minute) = digits(m)
                {
                    return Some(Zone::Offset {
                        negative,
                        hour,
                        minute,
                    });
                }
            }
            // minutes group absent
            None => {
                return Some(Zone::Offset {
                    negative,
                    hour,
                    minute: 0,
                });
            }
        }
    }
    None
}

fn match_date_time(s: &[u8]) -> Option<DateTime> {
    if s.len() < 10 || s[4] != b'-' || s[7] != b'-' {
        return None;
    }
    let year = digits(&s[0..4])?;
    let month = digits(&s[5..7])?;
    let day = digits(&s[8..10])?;
    let rest = &s[10..];

    let found = |hms: (i64, i64, i64), zone: Zone| DateTime {
        year,
        month,
        day,
        hour: hms.0,
        minute: hms.1,
        second: hms.2,
        zone,
    };

    // The time group is optional and greedy: try it first, then without it.
    if let Some((hour, minute, second, used)) = match_time(rest)
        && let Some(zone) = match_zone_and_end(&rest[used..])
    {
        return Some(found((hour, minute, second), zone));
    }
    let zone = match_zone_and_end(rest)?;
    Some(found((0, 0, 0), zone))
}

/// Days since 1970-01-01 of the first day of `month` (1-based, may be out of
/// range) of `year`, plus `day - 1`, using the proleptic Gregorian calendar
/// like glibc. Out-of-range months carry into the year.
fn days_from_civil_normalised(year: i64, month: i64, day: i64) -> i64 {
    let m0 = month - 1;
    let year = year + m0.div_euclid(12);
    let month = m0.rem_euclid(12) + 1;

    // Howard Hinnant's days_from_civil for day 1.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + (day - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Option<i64> {
        parse_date_time(s.as_bytes(), 0)
    }

    /// Values taken from the reference implementation (`parseDateTime` run
    /// with `TZ=UTC`).
    #[test]
    fn matches_reference_values() {
        let cases: &[(&str, Option<i64>)] = &[
            ("2010-01-02", Some(1262390400)),
            ("2010-01-02Z", Some(1262390400)),
            ("2010-01-02 03:04", Some(1262401440)),
            ("2010-01-02 03:04:05", Some(1262401445)),
            ("2010-01-01 03:04:05+12", Some(1262271845)),
            ("2010-01-01T03:04", Some(1262315040)),
            ("2010-01-01T03:04:05", Some(1262315045)),
            ("2010-01-01T03:04:05Z", Some(1262315045)),
            ("2010-01-01T03:04:05+12", Some(1262271845)),
            ("2010-01-01T00:04:05+5:30", Some(1262284445)),
            ("2010-01-01T03:04:05.6789", Some(1262315045)),
            ("2010-01-01T03:04:05.6789Z", Some(1262315045)),
            ("2010-13-45T25:61:61Z", Some(1297735321)),
            ("2010-00-00T00:00:00Z", Some(1259539200)),
            ("2010-01-01T03:04+530", Some(1262295240)),
            ("2010-01-01T03:04+0530", Some(1262295240)),
            ("2010-01-01T03:04+123", Some(1262310060)),
            ("2010-01-01T03:04-5", Some(1262333040)),
            ("2010-01-01T03:04 +05:30", Some(1262295240)),
            ("2010-01-01 +05:30", Some(1262284200)),
            ("2010-01-02-05", Some(1262408400)),
            ("2010-01-02 -0500", Some(1262408400)),
            ("1970-01-01T00:00:00Z", Some(0)),
            ("1969-12-31T23:59:59Z", Some(-1)),
            ("0000-01-01T00:00:00Z", Some(-62167219200)),
            ("9999-12-31T23:59:59Z", Some(253402300799)),
            ("2010-01-01T3:04:05Z", Some(1262315045)),
            ("2010-01-01T003:04:05Z", None),
            ("2010-01-01T03:4:05Z", None),
            ("2010-01-01T03:04:5Z", None),
            ("2010-01-01T03:04:05.Z", None),
            ("2010-1-1", None),
            ("10-01-01", None),
            ("2010-01-01T", None),
            ("2010-01-01T03", None),
            ("2010-01-01T03:04:05:06", None),
            ("2010-01-01T03:04:05+", None),
            ("2010-01-01T03:04:05+1:2", None),
            ("2010-01-01T03:04:05+12:3", None),
            ("2010-01-01T03:04:05+12345", None),
            ("2010-01-01T03:04:05 +12", Some(1262271845)),
            ("2010-01-01T03:04:05  +12", None),
            ("2010-01-01T03:04:05z", None),
            ("2010-01-01t03:04:05", None),
            ("-5-3", None),
            ("5-", None),
            ("a-b", None),
            ("", None),
        ];
        for (input, want) in cases {
            assert_eq!(p(input), *want, "{input:?}");
        }
    }

    #[test]
    fn local_offset_applies_only_to_zoneless_strings() {
        // Reference run with TZ=Asia/Seoul (UTC+9).
        let seoul = 9 * 3600;
        let local = |s: &str| parse_date_time(s.as_bytes(), seoul);
        assert_eq!(local("2010-01-02"), Some(1262358000));
        assert_eq!(local("2010-01-01T03:04:05"), Some(1262282645));
        assert_eq!(local("2010-01-01T03:04:05.6789"), Some(1262282645));
        // explicit zones ignore it
        assert_eq!(local("2010-01-02Z"), Some(1262390400));
        assert_eq!(local("2010-01-01T03:04:05+12"), Some(1262271845));
        // New York in winter (UTC-5)
        assert_eq!(parse_date_time(b"2010-01-02", -5 * 3600), Some(1262408400));
    }

    #[test]
    fn civil_days_are_gregorian() {
        assert_eq!(days_from_civil_normalised(1970, 1, 1), 0);
        assert_eq!(days_from_civil_normalised(2000, 3, 1), 11017);
        assert_eq!(days_from_civil_normalised(1900, 3, 1), -25508);
        // month and day overflow carry
        assert_eq!(
            days_from_civil_normalised(2010, 13, 1),
            days_from_civil_normalised(2011, 1, 1)
        );
        assert_eq!(
            days_from_civil_normalised(2010, 0, 1),
            days_from_civil_normalised(2009, 12, 1)
        );
        assert_eq!(
            days_from_civil_normalised(2010, 1, 0),
            days_from_civil_normalised(2009, 12, 31)
        );
    }

    #[test]
    fn rejects_trailing_garbage_and_non_ascii_digits() {
        assert_eq!(p("2010-01-02 "), None);
        assert_eq!(p("2010-01-02\n"), None);
        assert_eq!(p("2010-01-02Zx"), None);
        assert_eq!(p("２０１０-01-02"), None);
        assert_eq!(p("2010-01-02T03:04:05\0"), None);
    }
}
