// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Calendar arithmetic, the `strftime` subset Gource's `--date-format` can
//! use, and `SDLAppSettings::parseDateTime` (`core/settings.cpp`).
//!
//! Gource formats with `localtime` + `strftime` in the C locale (it never
//! calls `setlocale`), so names are English and `%X` / `%x` / `%c` use the C
//! locale layouts. The process time zone becomes an offset per timestamp
//! (`Settings::utc_offset_at`, local = UTC + offset): a fixed offset plus the
//! host's table of offset changes, which carries DST when the host fills it.
//! `%Z` is best effort: `UTC` for a zero offset, otherwise the numeric
//! abbreviation tzdata uses for zones without a name (`+09`, `+0530`).
//!
//! Supported conversions: `%a %A %b %B %c %C %d %D %e %F %h %H %I %j %k %l
//! %m %M %n %p %P %r %R %s %S %t %T %u %w %x %X %y %Y %z %Z %%`. Anything
//! else is copied through unchanged.

use std::fmt::Write;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Broken-down time (`struct tm`, without DST).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tm {
    pub(crate) year: i64,
    /// 0..=11
    pub(crate) mon: u32,
    /// 1..=31
    pub(crate) mday: u32,
    pub(crate) hour: u32,
    pub(crate) min: u32,
    pub(crate) sec: u32,
    /// 0 = Sunday
    pub(crate) wday: u32,
    /// 0..=365
    pub(crate) yday: u32,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..=12).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Proleptic Gregorian date of a day count since 1970-01-01.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// `gmtime` of `ts` seconds since the epoch.
pub(crate) fn tm_from_unix(ts: i64) -> Tm {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (year, month, mday) = civil_from_days(days);
    let jan1 = days_from_civil(year, 1, 1);
    Tm {
        year,
        mon: month - 1,
        mday,
        hour: (secs / 3600) as u32,
        min: ((secs / 60) % 60) as u32,
        sec: (secs % 60) as u32,
        wday: (days + 4).rem_euclid(7) as u32,
        yday: (days - jan1) as u32,
    }
}

/// `mktime_utc`: seconds since the epoch of a broken-down UTC time whose
/// fields may be out of range (normalised like `timegm`).
pub(crate) fn timegm(year: i64, mon0: i64, mday: i64, hour: i64, min: i64, sec: i64) -> i64 {
    let y = year + mon0.div_euclid(12);
    let m = mon0.rem_euclid(12) + 1;
    let days = days_from_civil(y, m, 1) + (mday - 1);
    days * 86400 + hour * 3600 + min * 60 + sec
}

fn push_zone_name(out: &mut String, offset_minutes: i32) {
    if offset_minutes == 0 {
        out.push_str("UTC");
        return;
    }
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let a = offset_minutes.unsigned_abs();
    let _ = write!(out, "{}{:02}", sign, a / 60);
    if !a.is_multiple_of(60) {
        let _ = write!(out, "{:02}", a % 60);
    }
}

fn hour12(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        h => h,
    }
}

/// `strftime(fmt, localtime(ts))` appended to `out`, with local time =
/// UTC + `offset_minutes`. Writes without allocating once `out` has room.
pub(crate) fn format_time(out: &mut String, fmt: &str, ts: i64, offset_minutes: i32) {
    let local = ts.wrapping_add(offset_minutes as i64 * 60);
    let tm = tm_from_unix(local);
    format_tm(out, fmt, &tm, ts, offset_minutes);
}

fn format_tm(out: &mut String, fmt: &str, tm: &Tm, ts: i64, offset_minutes: i32) {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        let wday = WEEKDAYS[tm.wday as usize];
        let month = MONTHS[tm.mon as usize];
        let _ = match spec {
            'a' => write!(out, "{}", &wday[..3]),
            'A' => write!(out, "{wday}"),
            'b' | 'h' => write!(out, "{}", &month[..3]),
            'B' => write!(out, "{month}"),
            'c' => {
                format_tm(out, "%a %b %e %H:%M:%S %Y", tm, ts, offset_minutes);
                Ok(())
            }
            'C' => write!(out, "{:02}", tm.year.div_euclid(100)),
            'd' => write!(out, "{:02}", tm.mday),
            'D' | 'x' => write!(
                out,
                "{:02}/{:02}/{:02}",
                tm.mon + 1,
                tm.mday,
                tm.year.rem_euclid(100)
            ),
            'e' => write!(out, "{:2}", tm.mday),
            'F' => write!(out, "{}-{:02}-{:02}", tm.year, tm.mon + 1, tm.mday),
            'H' => write!(out, "{:02}", tm.hour),
            'I' => write!(out, "{:02}", hour12(tm.hour)),
            'j' => write!(out, "{:03}", tm.yday + 1),
            'k' => write!(out, "{:2}", tm.hour),
            'l' => write!(out, "{:2}", hour12(tm.hour)),
            'm' => write!(out, "{:02}", tm.mon + 1),
            'M' => write!(out, "{:02}", tm.min),
            'n' => {
                out.push('\n');
                Ok(())
            }
            'p' => write!(out, "{}", if tm.hour < 12 { "AM" } else { "PM" }),
            'P' => write!(out, "{}", if tm.hour < 12 { "am" } else { "pm" }),
            'r' => write!(
                out,
                "{:02}:{:02}:{:02} {}",
                hour12(tm.hour),
                tm.min,
                tm.sec,
                if tm.hour < 12 { "AM" } else { "PM" }
            ),
            'R' => write!(out, "{:02}:{:02}", tm.hour, tm.min),
            's' => write!(out, "{ts}"),
            'S' => write!(out, "{:02}", tm.sec),
            't' => write!(out, "\t"),
            'T' | 'X' => write!(out, "{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
            'u' => write!(out, "{}", if tm.wday == 0 { 7 } else { tm.wday }),
            'w' => write!(out, "{}", tm.wday),
            'y' => write!(out, "{:02}", tm.year.rem_euclid(100)),
            'Y' => write!(out, "{}", tm.year),
            'z' => {
                let sign = if offset_minutes < 0 { '-' } else { '+' };
                let a = offset_minutes.unsigned_abs();
                write!(out, "{}{:02}{:02}", sign, a / 60, a % 60)
            }
            'Z' => {
                push_zone_name(out, offset_minutes);
                Ok(())
            }
            '%' => write!(out, "%"),
            other => write!(out, "%{other}"),
        };
    }
}

/// C `atoi` on an ASCII digit string (the regex guarantees digits).
fn atoi(s: &str) -> i64 {
    let mut v: i64 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        v = v * 10 + (b - b'0') as i64;
    }
    v
}

/// `SDLAppSettings::parseDateTime(datetime, timestamp)`: SQL-style and ISO
/// 8601 dates with optional time, `Z` or a `±h[h][[:]mm]` zone. Without a
/// zone the date is local time: UTC + `offset_at(utc timestamp)` minutes.
pub(crate) fn parse_date_time(datetime: &str, offset_at: impl Fn(i64) -> i32) -> Option<i64> {
    let re = regex_lite::Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{1,2}):(\d{2})(?::(\d{2}(?:\.\d+)?))?)?(Z| ?([+-])(\d{1,2})(?::?(\d{2}))?)?$",
    )
    .expect("valid date regex");
    let caps = re.captures(datetime)?;

    // Core's Regex::match returns groups 1..rc, where rc - 1 is the highest
    // group that took part in the match; unmatched groups below it are "".
    let size = (1..caps.len())
        .rev()
        .find(|&i| caps.get(i).is_some())
        .unwrap_or(0);
    let group = |i: usize| caps.get(i + 1).map(|m| m.as_str()).unwrap_or("");

    if size < 3 {
        return None;
    }

    let year = atoi(group(0)) - 1900;
    let mon = atoi(group(1)) - 1;
    let mday = atoi(group(2));
    let (mut hour, mut min, mut sec) = (0, 0, 0);

    if size >= 5 && !group(3).is_empty() {
        hour = atoi(group(3));
        min = atoi(group(4));
        if size >= 6 {
            sec = atoi(group(5));
        }
    }

    let utc = timegm(year + 1900, mon, mday, hour, min, sec);

    if size == 7 && group(6) == "Z" {
        Some(utc)
    } else if size >= 9 {
        let tz_hour = atoi(group(8));
        let tz_min = if size >= 10 { atoi(group(9)) } else { 0 };
        let mut tz_offset = tz_hour * 3600 + tz_min * 60;
        if group(7) == "-" {
            tz_offset = -tz_offset;
        }
        Some(utc - tz_offset)
    } else {
        // mktime: local time; the offset in force at that instant (found
        // from a first guess with the offset at the local wall time)
        let guess = utc - offset_at(utc) as i64 * 60;
        Some(utc - offset_at(guess) as i64 * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(f: &str, ts: i64, off: i32) -> String {
        let mut s = String::new();
        format_time(&mut s, f, ts, off);
        s
    }

    #[test]
    fn civil_round_trip() {
        for days in [-800_000i64, -1, 0, 1, 59, 365, 11_016, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m as i64, d as i64), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert!(is_leap(2000) && !is_leap(1900) && is_leap(2024));
    }

    #[test]
    fn default_gource_date_format() {
        // 1277787455 = Tue Jun 29 04:57:35 UTC 2010
        assert_eq!(
            fmt("%A, %d %B, %Y %X", 1277787455, 0),
            "Tuesday, 29 June, 2010 04:57:35"
        );
        assert_eq!(
            fmt("%A, %d %B, %Y %X", 1277787455, 12 * 60),
            "Tuesday, 29 June, 2010 16:57:35"
        );
        // In UTC+21 it is already the 30th; in UTC-5 still the 28th.
        assert_eq!(
            fmt("%A, %d %B, %Y %X", 1277787455, 21 * 60),
            "Wednesday, 30 June, 2010 01:57:35"
        );
        assert_eq!(
            fmt("%A, %d %B, %Y %X", 1277787455, -5 * 60),
            "Monday, 28 June, 2010 23:57:35"
        );
        assert_eq!(
            fmt("%a %b %e %I:%M %p %y %m %j", 1277787455, 0),
            "Tue Jun 29 04:57 AM 10 06 180"
        );
        assert_eq!(
            fmt("%x|%c|%%|%q", 0, 0),
            "01/01/70|Thu Jan  1 00:00:00 1970|%|%q"
        );
        assert_eq!(fmt("%z %Z", 0, 330), "+0530 +0530");
        assert_eq!(fmt("%z %Z", 0, -540), "-0900 -09");
        assert_eq!(fmt("%Z", 0, 0), "UTC");
        assert_eq!(fmt("%u %w", 3 * 86400, 0), "7 0");
        // before the epoch
        assert_eq!(fmt("%Y-%m-%d %H:%M:%S", -1, 0), "1969-12-31 23:59:59");
    }

    #[test]
    fn parses_gource_date_syntax() {
        let d = |s: &str| parse_date_time(s, |_| 0);
        assert_eq!(d("2010-01-02"), Some(timegm(2010, 0, 2, 0, 0, 0)));
        assert_eq!(d("2010-01-02Z"), Some(timegm(2010, 0, 2, 0, 0, 0)));
        assert_eq!(d("2010-01-02 03:04"), Some(timegm(2010, 0, 2, 3, 4, 0)));
        assert_eq!(d("2010-01-02 03:04:05"), Some(timegm(2010, 0, 2, 3, 4, 5)));
        assert_eq!(
            d("2010-01-01T03:04:05.6789"),
            Some(timegm(2010, 0, 1, 3, 4, 5))
        );
        assert_eq!(
            d("2010-01-01 03:04:05+12"),
            Some(timegm(2010, 0, 1, 3, 4, 5) - 12 * 3600)
        );
        assert_eq!(
            d("2010-01-01T00:04:05+5:30"),
            Some(timegm(2010, 0, 1, 0, 4, 5) - (5 * 3600 + 30 * 60))
        );
        assert_eq!(
            d("2010-01-01T00:04:05 -0800"),
            Some(timegm(2010, 0, 1, 0, 4, 5) + 8 * 3600)
        );
        assert_eq!(d("2010-1-01"), None);
        assert_eq!(d("yesterday"), None);
        // local time without a zone
        assert_eq!(
            parse_date_time("2010-01-02", |_| 60),
            Some(timegm(2010, 0, 2, 0, 0, 0) - 3600)
        );
        // the offset in force at that date (UTC+1, UTC+2 from June 2010)
        let june = timegm(2010, 5, 1, 0, 0, 0);
        let dst = |ts: i64| if ts >= june { 120 } else { 60 };
        assert_eq!(
            parse_date_time("2010-01-02 12:00", dst),
            Some(timegm(2010, 0, 2, 11, 0, 0))
        );
        assert_eq!(
            parse_date_time("2010-07-02 12:00", dst),
            Some(timegm(2010, 6, 2, 10, 0, 0))
        );
        // mktime normalises out-of-range fields
        assert_eq!(d("2010-13-01"), Some(timegm(2011, 0, 1, 0, 0, 0)));
        assert_eq!(timegm(1970, 0, 1, 0, 0, 0), 0);
        assert_eq!(timegm(2010, 5, 29, 4, 57, 35), 1277787455);
    }
}
