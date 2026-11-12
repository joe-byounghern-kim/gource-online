// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Custom log format parser (`formats/custom.cpp`).
//!
//! One line per file change:
//!
//! ```text
//! timestamp|username|A/M/D|/path/to/file[|RRGGBB][|meta]
//! ```
//!
//! [`CustomLogParser`] is a push parser: feed it byte chunks of any size and
//! it emits one [`ParsedCommit`] per commit to a [`CommitSink`]. The result
//! does not depend on where the chunk boundaries fall (inside a line, inside
//! a multi-byte UTF-8 sequence, between `\r` and `\n`).
//!
//! # What is reproduced
//!
//! The reading logic of `core/seeklog.cpp`, `formats/commitlog.cpp` and
//! `formats/custom.cpp`:
//!
//! - **Lines.** Split at `\n`; one trailing `\r` is removed
//!   (`std::getline` + `SeekLog::getNextLine`). A final line without a
//!   newline is a line like any other once [`CustomLogParser::finish`] runs.
//!   Blank lines are lines (and malformed, see below).
//! - **Fields.** The `custom_regex`: an optional UTF-8 byte order mark at the
//!   start of *each* line, then `timestamp|username|action|path`, where the
//!   timestamp is one or more non-`|` bytes, the username may be empty, the
//!   action is empty or exactly one of `A`, `D`, `M`, and the path is one or
//!   more non-`|` bytes. An optional fifth column `[#]RRGGBB` is recognised
//!   when it starts right after the path's `|` and has six hex digits;
//!   anything after the match is ignored (`1|u|A|/a|b` is the file `/a`, and
//!   `1|u|A|/a|12345` has no colour).
//! - **Meta field** (an extension; C++ ignores it). The text after the
//!   path's `|` is split at its next `|`: the left part is the colour slot,
//!   read as above, and the right part is the meta field ([`crate::meta`]).
//!   Its AI credits are collected over all lines of a commit (the union,
//!   because two real commits of one author with equal timestamps form one
//!   custom-log commit) and handed out in canonical order
//!   ([`crate::canonicalize`]). Malformed meta never costs the line; it is
//!   counted in [`ParseStats::bad_meta`]. After
//!   [`gource_core::events::MAX_ATTR_SETS`] distinct credit sets, further
//!   new sets are dropped (and counted).
//! - **Timestamp.** A value with a `-` after its first byte is a date string
//!   (`datetime` module). Anything else is read with `atoll`: leading white
//!   space, an optional sign, digits, then anything; saturating on overflow.
//!   A result of `0` is only accepted for the text `0` itself (so `00`, `+0`
//!   and `abc` are rejected, `0` and `-2` are fine).
//! - **Defaults.** Empty username becomes `Unknown`, empty action `A`.
//! - **Grouping.** Consecutive lines with the same timestamp and the same
//!   (raw, unfiltered) username form one commit. Output order is input
//!   order; nothing is sorted or de-duplicated.
//! - **Malformed lines.** A line that does not match the format or has a bad
//!   timestamp is dropped, counted in [`ParseStats::skipped_lines`], and it
//!   *ends the commit in progress*: the commit read so far is emitted and
//!   the next valid line starts a new one even if it has the same timestamp
//!   and user (`parseCommitEntry` returns false, `parseCommit` returns the
//!   files collected so far).
//! - **Text.** Usernames and file names go through `RCommitLog::filter_utf8`
//!   (`utf8filter` module); file names get a leading `/` when missing. The
//!   explicit colour is returned as `[r, g, b]`; hashing the extension for
//!   lines without one is the simulation's job.
//!
//! # Deliberate differences
//!
//! - Gource's `readLog` loop drops the last commit when the file does not end
//!   in a newline and that last line starts a new commit (the stream's EOF
//!   flag is already set). The last line is parsed here, as the spec asks.
//! - `SeekLog` builds its in-memory log with `std::string(char*)`, which
//!   cuts the file at the first NUL byte. NUL is an ordinary byte here.
//! - A date without a zone is local time in C++; see
//!   [`CustomLogParser::with_local_utc_offset`].
//!
//! # Allocation
//!
//! No `String` is created per line. Lines are parsed in place from the chunk
//! (or from a reused carry buffer when a line spans chunks) and the commit in
//! progress lives in buffers that are cleared, not freed, between commits.

use gource_core::events::{ChangeEvent, ChangeKind};

use crate::attrs::{self, OwnedAttrs, SetCap};
use crate::datetime::parse_date_time;
use crate::meta::{MetaReader, OwnedPrLifecycle};
use crate::utf8filter::{filter_utf8, filter_utf8_into};
use crate::{
    CommitSink, ParseStats, ParsedAttribution, ParsedCommit, ParsedFile, ParsedPrChange,
    ParsedPrPayload, canonicalize,
};

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Streaming parser for Gource's custom log format (`formats/custom.cpp`).
#[derive(Debug, Default)]
pub struct CustomLogParser {
    stats: ParseStats,
    local_utc_offset: i64,

    /// Start of the current line when it began in an earlier chunk.
    carry: Vec<u8>,
    carry_active: bool,
    carry_offset: u64,

    pending: Pending,
    /// Holds the filtered username when the raw one is not valid UTF-8.
    user_buf: String,
    /// Empty between commits; only its allocation is kept, for the
    /// `ParsedCommit::files` slice of the next one (see `recycle`).
    files_scratch: Vec<ParsedFile<'static>>,
    /// The same for `ParsedCommit::attrs`.
    attrs_scratch: Vec<ParsedAttribution<'static>>,
    meta: MetaReader,
    cap: SetCap,
}

impl CustomLogParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offset applied to date strings that carry no zone designator, in
    /// seconds east of UTC. C++ uses the machine's local time zone for them;
    /// that is not available (or deterministic) everywhere this parser runs,
    /// so the default is UTC. A browser can pass
    /// `-new Date().getTimezoneOffset() * 60`.
    pub fn with_local_utc_offset(mut self, seconds_east_of_utc: i32) -> Self {
        self.local_utc_offset = i64::from(seconds_east_of_utc);
        self
    }

    /// Feed the next chunk; complete commits are emitted to `sink`.
    ///
    /// A commit is complete when a line with another timestamp or user, or a
    /// malformed line, follows it, so the most recent commit is held back
    /// until the next chunk or [`CustomLogParser::finish`].
    pub fn push(&mut self, bytes: &[u8], sink: &mut dyn CommitSink) {
        let base = self.stats.bytes;
        self.stats.bytes += bytes.len() as u64;

        let mut start = 0;
        while let Some(rel) = memchr(b'\n', &bytes[start..]) {
            let end = start + rel;
            if self.carry_active {
                self.carry.extend_from_slice(&bytes[start..end]);
                self.finish_carried_line(sink);
            } else {
                self.line(&bytes[start..end], base + start as u64, sink);
            }
            start = end + 1;
        }

        if start < bytes.len() {
            if !self.carry_active {
                self.carry_active = true;
                self.carry_offset = base + start as u64;
                self.carry.clear();
            }
            self.carry.extend_from_slice(&bytes[start..]);
        }
    }

    /// Flush the last (possibly unterminated) line and pending commit.
    pub fn finish(&mut self, sink: &mut dyn CommitSink) {
        if self.carry_active {
            self.finish_carried_line(sink);
        }
        self.flush(sink);
    }

    pub fn stats(&self) -> ParseStats {
        self.stats
    }

    fn finish_carried_line(&mut self, sink: &mut dyn CommitSink) {
        let line = std::mem::take(&mut self.carry);
        self.line(&line, self.carry_offset, sink);
        self.carry = line;
        self.carry.clear();
        self.carry_active = false;
    }

    /// One complete line (without its `\n`) starting at `offset`.
    fn line(&mut self, raw: &[u8], offset: u64, sink: &mut dyn CommitSink) {
        self.stats.lines += 1;

        // SeekLog::getNextLine removes one trailing carriage return.
        let line = match raw.split_last() {
            Some((b'\r', head)) => head,
            _ => raw,
        };

        let Some(rec) = parse_line(line, self.local_utc_offset) else {
            // parseCommitEntry() returned false: the commit in progress ends
            // here and the line is gone.
            self.stats.skipped_lines += 1;
            self.flush(sink);
            return;
        };

        let same_commit = !self.pending.files.is_empty()
            && self.pending.timestamp == rec.timestamp
            && self.pending.user == rec.username;
        if !same_commit {
            self.flush(sink);
            self.pending.timestamp = rec.timestamp;
            self.pending.byte_offset = offset;
            self.pending.user.clear();
            self.pending.user.extend_from_slice(rec.username);
        }
        let mut event = ChangeEvent::Commit;
        let mut pr = None;
        if let Some(field) = rec.meta {
            self.stats.bad_meta +=
                self.meta
                    .read_line(field, &mut self.pending.attrs, Some(rec.kind));
            if let Some(update) = self.meta.coverage() {
                // Frozen stream metadata does not split/reorder equal-time commits.
                sink.pr_log(update);
            }
            if self.meta.rejected_pr() {
                self.stats.skipped_lines += 1;
                return;
            }
            event = self.meta.event();
            pr = self.meta.take_pr();
        }
        self.pending.add_file(&rec, event);
        if let Some(payload) = pr {
            self.pending
                .pr
                .push((self.pending.files.len() as u32 - 1, payload));
        }
    }

    /// Emit the commit in progress, if any.
    fn flush(&mut self, sink: &mut dyn CommitSink) {
        if self.pending.files.is_empty() {
            return;
        }

        let pending = &self.pending;
        let username = filter_utf8(&pending.user, &mut self.user_buf);

        let mut files = recycle(std::mem::take(&mut self.files_scratch));
        let mut start = 0;
        for f in &pending.files {
            files.push(ParsedFile {
                path: &pending.paths[start..f.end],
                kind: f.kind,
                colour: f.colour,
                event: f.event,
            });
            start = f.end;
        }

        let mut attrs = attrs::recycle(std::mem::take(&mut self.attrs_scratch));
        if !pending.attrs.is_empty() {
            pending.attrs.extend_into(&mut attrs);
            canonicalize(&mut attrs);
            if !self.cap.admit(&attrs) {
                attrs.clear();
                self.stats.bad_meta += 1;
            }
        }

        let commit = ParsedCommit {
            timestamp: pending.timestamp,
            username,
            files: &files,
            attrs: &attrs,
            byte_offset: pending.byte_offset,
        };
        if pending.pr.is_empty() {
            sink.commit(&commit);
        } else {
            let pr: Vec<ParsedPrChange<'_>> = pending
                .pr
                .iter()
                .map(|(file_index, payload)| ParsedPrChange {
                    file_index: *file_index,
                    payload: ParsedPrPayload::Lifecycle(payload.borrowed()),
                })
                .collect();
            sink.commit_pr(&commit, &pr);
        }

        self.stats.commits += 1;
        self.stats.files += files.len() as u64;
        self.files_scratch = recycle(files);
        self.attrs_scratch = attrs::recycle(attrs);
        self.pending.clear();
    }
}

/// Reuse the allocation of a `Vec` of borrowed entries for another lifetime.
/// The vector is emptied first, so nothing borrowed survives; the standard
/// library's in-place `collect` keeps the buffer.
fn recycle<'a, 'b>(mut v: Vec<ParsedFile<'a>>) -> Vec<ParsedFile<'b>> {
    v.clear();
    v.into_iter().map_while(|_| None).collect()
}

/// The commit being assembled from consecutive lines.
#[derive(Debug, Default)]
struct Pending {
    timestamp: i64,
    byte_offset: u64,
    /// Raw username bytes; commits are grouped on these, filtering comes
    /// after (`RCommit::postprocess`).
    user: Vec<u8>,
    /// Filtered file names, back to back.
    paths: String,
    files: Vec<PendingFile>,
    /// AI credits of every line of the commit, not yet canonical.
    attrs: OwnedAttrs,
    pr: Vec<(u32, OwnedPrLifecycle)>,
}

#[derive(Copy, Clone, Debug)]
struct PendingFile {
    /// End of this file's name in `Pending::paths`.
    end: usize,
    kind: ChangeKind,
    colour: Option<[u8; 3]>,
    event: ChangeEvent,
}

impl Pending {
    fn clear(&mut self) {
        self.user.clear();
        self.paths.clear();
        self.files.clear();
        self.attrs.clear();
        self.pr.clear();
    }

    /// `RCommitFile::RCommitFile`: filter the name, root it with `/`.
    fn add_file(&mut self, rec: &Record<'_>, event: ChangeEvent) {
        // A leading '/' is valid ASCII and survives filtering, while any
        // other first byte stays something else (or becomes '?').
        if rec.path.first() != Some(&b'/') {
            self.paths.push('/');
        }
        filter_utf8_into(rec.path, &mut self.paths);
        self.files.push(PendingFile {
            end: self.paths.len(),
            kind: rec.kind,
            colour: rec.colour,
            event,
        });
    }
}

/// The fields of one accepted line, borrowed from it.
struct Record<'a> {
    timestamp: i64,
    /// Raw username, `Unknown` when the column is empty.
    username: &'a [u8],
    kind: ChangeKind,
    /// Raw path column (never empty).
    path: &'a [u8],
    colour: Option<[u8; 3]>,
    /// The meta field: everything after the colour slot's `|`.
    meta: Option<&'a [u8]>,
}

/// Match `custom_regex` and the timestamp rules of
/// `CustomLog::parseCommitEntry`. `None` for a malformed line.
fn parse_line(line: &[u8], local_utc_offset: i64) -> Option<Record<'_>> {
    // `^(?:\xEF\xBB\xBF)?`: a BOM followed directly by '|' leaves the
    // timestamp empty, and the regex then falls back to reading the BOM as
    // the timestamp, which `atoll` turns into 0 and the caller rejects.
    let rest = line.strip_prefix(&BOM).unwrap_or(line);

    // `([^|]+)\|([^|]*)\|([ADM]?)\|([^|]+)`
    let p = memchr(b'|', rest)?;
    let (stamp, rest) = (&rest[..p], &rest[p + 1..]);
    if stamp.is_empty() {
        return None;
    }
    let p = memchr(b'|', rest)?;
    let (user, rest) = (&rest[..p], &rest[p + 1..]);
    let (kind, rest) = match rest {
        [b'|', tail @ ..] => (ChangeKind::Add, tail),
        [b'A', b'|', tail @ ..] => (ChangeKind::Add, tail),
        [b'M', b'|', tail @ ..] => (ChangeKind::Modify, tail),
        [b'D', b'|', tail @ ..] => (ChangeKind::Delete, tail),
        _ => return None,
    };
    let (path, tail) = match memchr(b'|', rest) {
        Some(p) => (&rest[..p], Some(&rest[p + 1..])),
        None => (rest, None),
    };
    if path.is_empty() {
        return None;
    }

    let timestamp = parse_timestamp(stamp, local_utc_offset)?;
    // The colour slot ends at the next '|'; reading it on its own gives the
    // same result as the regex on the whole tail, because a '|' within the
    // seven bytes `#RRGGBB` fails both.
    let (slot, meta) = match tail {
        Some(t) => match memchr(b'|', t) {
            Some(p) => (Some(&t[..p]), Some(&t[p + 1..])),
            None => (Some(t), None),
        },
        None => (None, None),
    };
    Some(Record {
        timestamp,
        username: if user.is_empty() { b"Unknown" } else { user },
        kind,
        path,
        // `(?:\|#?([a-fA-F0-9]{6}))?`
        colour: slot.and_then(parse_colour),
        meta,
    })
}

fn parse_timestamp(field: &[u8], local_utc_offset: i64) -> Option<i64> {
    // Plain decimal numbers are nearly all of them; 18 digits cannot overflow.
    if (1..=18).contains(&field.len()) && field.iter().all(u8::is_ascii_digit) {
        let value = field
            .iter()
            .fold(0i64, |v, &b| v * 10 + i64::from(b - b'0'));
        return (value != 0 || field == b"0").then_some(value);
    }

    // Allow timestamp to be a string
    if field.len() > 1 && field[1..].contains(&b'-') {
        return parse_date_time(field, local_utc_offset);
    }

    let value = atoll(field);
    (value != 0 || field == b"0").then_some(value)
}

/// `atoll` in the C locale: skip white space, optional sign, decimal digits,
/// stop at anything else (including NUL, the end of the C string); saturate.
fn atoll(s: &[u8]) -> i64 {
    let s = s.split(|&b| b == 0).next().unwrap_or_default();
    let mut s = s
        .iter()
        .copied()
        .skip_while(|b| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        .peekable();

    let negative = match s.peek() {
        Some(b'-') => {
            s.next();
            true
        }
        Some(b'+') => {
            s.next();
            false
        }
        _ => false,
    };

    let mut magnitude: u64 = 0;
    for b in s.take_while(u8::is_ascii_digit) {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(b - b'0'));
    }

    if negative {
        i64::try_from(magnitude).map_or(i64::MIN, |m| -m)
    } else {
        i64::try_from(magnitude).unwrap_or(i64::MAX)
    }
}

/// `[#]RRGGBB` at the start of `s`; whatever follows is ignored.
fn parse_colour(s: &[u8]) -> Option<[u8; 3]> {
    let s = s.strip_prefix(b"#").unwrap_or(s);
    let h = s.get(..6)?;
    let mut nibbles = [0u8; 6];
    for (n, &c) in nibbles.iter_mut().zip(h) {
        *n = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
    }
    Some([
        nibbles[0] << 4 | nibbles[1],
        nibbles[2] << 4 | nibbles[3],
        nibbles[4] << 4 | nibbles[5],
    ])
}

/// Position of the first `needle` in `haystack`, eight bytes at a time.
#[inline]
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    const LO: u64 = 0x0101_0101_0101_0101;
    const HI: u64 = 0x8080_8080_8080_8080;

    let pattern = LO * u64::from(needle);
    let (words, tail) = haystack.as_chunks::<8>();
    for (i, w) in words.iter().enumerate() {
        // Bytes equal to `needle` become zero; the lowest flagged byte of
        // the classic "has zero byte" test is always a true zero.
        let x = u64::from_le_bytes(*w) ^ pattern;
        let zero = x.wrapping_sub(LO) & !x & HI;
        if zero != 0 {
            return Some(i * 8 + (zero.trailing_zeros() / 8) as usize);
        }
    }
    tail.iter()
        .position(|&b| b == needle)
        .map(|p| words.len() * 8 + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memchr_matches_position_everywhere() {
        let mut data = vec![b'a'; 100];
        for needle_at in 0..100 {
            data[needle_at] = b'|';
            assert_eq!(memchr(b'|', &data), Some(needle_at));
            // later copies must not matter
            if needle_at + 3 < 100 {
                data[needle_at + 3] = b'|';
                assert_eq!(memchr(b'|', &data), Some(needle_at));
                data[needle_at + 3] = b'a';
            }
            data[needle_at] = b'a';
        }
        assert_eq!(memchr(b'|', &data), None);
        assert_eq!(memchr(b'|', b""), None);
        // bytes whose high bit is set and 0x01 neighbours of the needle
        let tricky = [0x80, 0xff, 0x7c ^ 1, 0x7c + 1, 0x7c, 0, 0, 0, 0x7c];
        assert_eq!(memchr(0x7c, &tricky), Some(4));
        assert_eq!(memchr(0x7c, &tricky[5..]), Some(3));
        assert_eq!(memchr(0xff, &tricky), Some(1));
        assert_eq!(memchr(0, &tricky), Some(5));
    }

    #[test]
    fn atoll_follows_the_c_library() {
        assert_eq!(atoll(b"0"), 0);
        assert_eq!(atoll(b"1700000000"), 1_700_000_000);
        assert_eq!(atoll(b"-2"), -2);
        assert_eq!(atoll(b"+7"), 7);
        assert_eq!(atoll(b"  \t\n\x0b\x0c\r42"), 42);
        assert_eq!(atoll(b"12abc"), 12);
        assert_eq!(atoll(b"12 34"), 12);
        assert_eq!(atoll(b"abc"), 0);
        assert_eq!(atoll(b""), 0);
        assert_eq!(atoll(b"-"), 0);
        assert_eq!(atoll(b"+-3"), 0);
        assert_eq!(atoll(b"1\x002"), 1);
        assert_eq!(atoll(b"\x00"), 0);
        assert_eq!(atoll(b"9223372036854775807"), i64::MAX);
        assert_eq!(atoll(b"9223372036854775808"), i64::MAX);
        assert_eq!(atoll(b"99999999999999999999999999"), i64::MAX);
        assert_eq!(atoll(b"-9223372036854775808"), i64::MIN);
        assert_eq!(atoll(b"-9223372036854775809"), i64::MIN);
        assert_eq!(atoll(b"-99999999999999999999999999"), i64::MIN);
        assert_eq!(atoll(b"0x10"), 0);
        assert_eq!(atoll("１２".as_bytes()), 0);
    }

    fn ts(field: &str) -> Option<i64> {
        parse_timestamp(field.as_bytes(), 0)
    }

    #[test]
    fn timestamp_acceptance_rules() {
        assert_eq!(ts("0"), Some(0));
        assert_eq!(ts("00"), None);
        assert_eq!(ts("+0"), None);
        assert_eq!(ts("-0"), None);
        assert_eq!(ts(" 0"), None);
        assert_eq!(ts("0abc"), None);
        assert_eq!(ts("abc"), None);
        assert_eq!(ts("1"), Some(1));
        assert_eq!(ts("-2"), Some(-2));
        assert_eq!(ts("+5"), Some(5));
        assert_eq!(ts(" 12"), Some(12));
        assert_eq!(ts("12abc"), Some(12));
        assert_eq!(ts("012"), Some(12));
        // a '-' after the first byte makes it a date, and a bad one is rejected
        assert_eq!(ts("5-"), None);
        assert_eq!(ts("not-a-timestamp"), None);
        assert_eq!(ts("1-2"), None);
        assert_eq!(ts("2010-01-02"), Some(1_262_390_400));
        // longer than 18 digits goes through atoll and saturates
        assert_eq!(ts("123456789012345678901"), Some(i64::MAX));
        assert_eq!(ts("-123456789012345678901"), Some(i64::MIN));
    }

    type Fields = (i64, String, ChangeKind, String, Option<[u8; 3]>);

    fn rec(line: &str) -> Option<Fields> {
        parse_line(line.as_bytes(), 0).map(|r| {
            (
                r.timestamp,
                String::from_utf8_lossy(r.username).into_owned(),
                r.kind,
                String::from_utf8_lossy(r.path).into_owned(),
                r.colour,
            )
        })
    }

    #[test]
    fn regex_equivalence_on_hand_picked_lines() {
        use ChangeKind::*;
        let ok = |t, u: &str, k, p: &str, c| Some((t, u.to_string(), k, p.to_string(), c));

        assert_eq!(rec("1|a|A|/p"), ok(1, "a", Add, "/p", None));
        assert_eq!(rec("1|a|M|p"), ok(1, "a", Modify, "p", None));
        assert_eq!(rec("1|a|D|p/"), ok(1, "a", Delete, "p/", None));
        // empty username and action default
        assert_eq!(rec("1||A|p"), ok(1, "Unknown", Add, "p", None));
        assert_eq!(rec("1|a||p"), ok(1, "a", Add, "p", None));
        // username may contain anything but '|'
        assert_eq!(rec("1|a b|A|p"), ok(1, "a b", Add, "p", None));
        // action must be exactly A, D or M
        assert_eq!(rec("1|a|X|p"), None);
        assert_eq!(rec("1|a|a|p"), None);
        assert_eq!(rec("1|a|AD|p"), None);
        assert_eq!(rec("1|a| |p"), None);
        // path must exist, and stops at the next '|'
        assert_eq!(rec("1|a|A|"), None);
        assert_eq!(rec("1|a|A"), None);
        assert_eq!(
            rec("1|a|A|src/has|delimiter.rs"),
            ok(1, "a", Add, "src/has", None)
        );
        // timestamp must exist
        assert_eq!(rec("|a|A|p"), None);
        assert_eq!(rec(""), None);
        assert_eq!(rec("no pipes"), None);
        // colours
        let red = Some([0xff, 0, 0]);
        assert_eq!(rec("1|a|A|p|ff0000"), ok(1, "a", Add, "p", red));
        assert_eq!(rec("1|a|A|p|#FF0000"), ok(1, "a", Add, "p", red));
        assert_eq!(rec("1|a|A|p|ff0000|more"), ok(1, "a", Add, "p", red));
        assert_eq!(rec("1|a|A|p|ff0000zzz"), ok(1, "a", Add, "p", red));
        assert_eq!(
            rec("1|a|A|p|0aBc1D"),
            ok(1, "a", Add, "p", Some([0x0a, 0xbc, 0x1d]))
        );
        assert_eq!(rec("1|a|A|p|12345"), ok(1, "a", Add, "p", None));
        assert_eq!(rec("1|a|A|p|#12345"), ok(1, "a", Add, "p", None));
        assert_eq!(rec("1|a|A|p|##123456"), ok(1, "a", Add, "p", None));
        assert_eq!(rec("1|a|A|p|gg0000"), ok(1, "a", Add, "p", None));
        assert_eq!(rec("1|a|A|p|"), ok(1, "a", Add, "p", None));
        assert_eq!(rec("1|a|A|p| ff0000"), ok(1, "a", Add, "p", None));
        // a bad timestamp rejects an otherwise matching line
        assert_eq!(rec("abc|a|A|p"), None);
        assert_eq!(rec("00|a|A|p"), None);
    }

    #[test]
    fn byte_order_mark_is_optional_on_every_line() {
        use ChangeKind::*;
        let ok = |t, u: &str, k, p: &str| Some((t, u.to_string(), k, p.to_string(), None));
        assert_eq!(rec("\u{feff}1|a|A|p"), ok(1, "a", Add, "p"));
        // a BOM with no timestamp after it is the same as no timestamp
        assert_eq!(rec("\u{feff}|a|A|p"), None);
        // a second BOM is part of the timestamp, which is not a number
        assert_eq!(rec("\u{feff}\u{feff}1|a|A|p"), None);
        // BOM inside other columns is just text
        assert_eq!(rec("1|\u{feff}a|A|p"), ok(1, "\u{feff}a", Add, "p"));
    }
}
