// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `git log --raw` parser (`formats/git.cpp`).
//!
//! Input is the output of Gource's log command:
//!
//! ```text
//! git log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature \
//!     --pretty=format:user:%aN%n%ct
//! ```
//!
//! or of the clone service's command with AI detection, which adds one meta
//! line after the timestamp (`%x1f` and `%x1e` are the bytes 0x1f and 0x1e):
//!
//! ```text
//! --pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f\
//!     %(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s
//! ```
//!
//! which looks like
//!
//! ```text
//! user:Jane Doe
//! 1234567890
//! :000000 100644 0000000 d00491f A→src/main.c
//! :100644 100644 d00491f 1191247 M→"sub/\303\274.txt"
//!
//! user:Merge Author
//! 1234567891
//! user:Jane Doe
//! 1234567892
//! :100644 000000 00750ed 0000000 D→src/old.c
//! ```
//!
//! (`→` is a tab). Blocks are separated by blank lines. A merge commit has
//! no raw lines, so its header is immediately followed by the next commit's
//! header in the same block; as in `GitCommitLog::parseCommit` the later
//! `user:` line replaces the earlier one, and commits without files are
//! dropped (`RCommit::isValid`).
//!
//! # The `m1:` line
//!
//! A line starting `m1:` directly after the timestamp is consumed whole (it
//! may contain a TAB: the subject is free text). The part before its first
//! `0x1e` is split on `0x1f` into author name and email, committer name and
//! email, and the trailers; the subject after `0x1e` is not used yet. The
//! raw identities and trailers go through [`crate::classify`], and only the
//! resulting agent ids and models are kept: no name, email or trailer text
//! leaves the parser. When the raw author name differs from the `user:`
//! name (a mailmap renamed it), an Agent verdict becomes Assist.
//!
//! The commit is left unattributed, with [`ParseStats::bad_meta`] counted,
//! when the line has no `0x1e` or fewer than five pieces, when an identity
//! piece contains a C0 control, when an email piece contains white space,
//! or when a trailer piece has no `:` (each means that a separator inside a
//! name shifted the fields). `user:` resets the pending attributions and
//! `m1:` assigns them, so a merge header's credits never reach the next
//! commit. An `m1:` line anywhere else is skipped. Output of the old
//! command has no such lines and parses as before.

use std::collections::HashSet;

use gource_core::events::{ChangeEvent, ChangeKind};

use crate::attrs::{self, OwnedAttrs, SetCap};
use crate::classify::{AttrBuf, CommitSignals, Identity, Trailer, classify};
use crate::{CommitSink, ParseStats, ParsedAttribution, ParsedCommit, ParsedFile};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum State {
    /// Between or inside commit blocks.
    #[default]
    Block,
    /// The line after `user:` is the timestamp, whatever it contains.
    Timestamp,
    /// After the timestamp: an `m1:` line here is the commit's meta line.
    Meta,
    /// Opt-in only: exactly one full source SHA after the optional m1 line.
    Source,
}

#[derive(Debug)]
struct SourceState {
    sha: String,
    seen: HashSet<String>,
    max_headers: usize,
    failed: bool,
}

/// Streaming parser for `git log --reverse --raw --no-renames
/// --pretty=format:user:%aN%n%ct` output (`formats/git.cpp`).
///
/// Chunks may split lines (and UTF-8 sequences) anywhere. Buffers are reused
/// across lines and commits, so steady-state parsing does not allocate.
///
/// Differences from the C++ parser, both deliberate: C-quoted paths
/// (`"sub/\303\274.txt"`) are decoded rather than only stripped of their
/// quotes, and [`ParsedCommit::byte_offset`] is the offset of the `user:`
/// line of the commit that owns the files (a merge header directly before it
/// does not count).
#[derive(Debug)]
pub struct GitRawParser {
    stats: ParseStats,
    state: State,
    /// Bytes of a line that started in an earlier chunk.
    partial: Vec<u8>,
    /// Input offset of the first byte of `partial`.
    partial_offset: u64,
    /// Current username (already UTF-8 filtered); empty = no user yet.
    username: String,
    timestamp: i64,
    /// Offset of the `user:` line that started the pending commit.
    commit_offset: u64,
    /// Concatenated, rooted, filtered paths of the pending commit.
    paths: String,
    /// `(start, end, kind)` ranges into `paths`.
    spans: Vec<(usize, usize, ChangeKind)>,
    /// Scratch buffer for unquoting.
    unquoted: Vec<u8>,
    /// Recycled `ParsedFile` storage (always empty between commits).
    files: Vec<ParsedFile<'static>>,
    /// `user:` headers seen, merges included.
    headers: u64,
    /// Most files buffered for one commit (`usize::MAX`: no limit).
    max_commit_files: usize,
    /// Most bytes buffered for one commit: its paths plus a line still being
    /// assembled across chunks (`usize::MAX`: no limit).
    max_commit_bytes: usize,
    /// A limit was hit; all further input is ignored.
    exceeded: bool,
    /// AI credits of the pending commit (from its `m1:` line).
    attrs: OwnedAttrs,
    /// Recycled storage for classification and `ParsedCommit::attrs`.
    attr_buf: AttrBuf<'static>,
    attr_views: Vec<ParsedAttribution<'static>>,
    trailers: Vec<Trailer<'static>>,
    /// The raw author name, UTF-8 filtered, for the mailmap comparison.
    raw_name: String,
    cap: SetCap,
    /// Credits dropped over the 4-entry cap.
    truncated: u64,
    source: Option<SourceState>,
}

impl Default for GitRawParser {
    fn default() -> Self {
        GitRawParser {
            stats: ParseStats::default(),
            state: State::default(),
            partial: Vec::new(),
            partial_offset: 0,
            username: String::new(),
            timestamp: 0,
            commit_offset: 0,
            paths: String::new(),
            spans: Vec::new(),
            unquoted: Vec::new(),
            files: Vec::new(),
            headers: 0,
            max_commit_files: usize::MAX,
            max_commit_bytes: usize::MAX,
            exceeded: false,
            attrs: OwnedAttrs::default(),
            attr_buf: AttrBuf::new(),
            attr_views: Vec::new(),
            trailers: Vec::new(),
            raw_name: String::new(),
            cap: SetCap::default(),
            truncated: 0,
            source: None,
        }
    }
}

impl GitRawParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// A parser that buffers at most `max_files` file changes and
    /// `max_bytes` bytes (paths plus an incomplete line) for one commit.
    ///
    /// The C++ parser has no such bound; it is for untrusted input such as a
    /// server reading `git log` of an arbitrary repository, where a single
    /// commit can list millions of files. When a limit is hit the parser
    /// stops: the pending commit and all later input are discarded, its
    /// buffers are released, and [`GitRawParser::limit_exceeded`] returns
    /// `true`. Commits emitted before that point are unaffected.
    pub fn with_commit_limits(max_files: usize, max_bytes: usize) -> Self {
        GitRawParser {
            max_commit_files: max_files,
            max_commit_bytes: max_bytes,
            ..Self::default()
        }
    }

    /// Whether a limit of [`GitRawParser::with_commit_limits`] was hit.
    pub fn limit_exceeded(&self) -> bool {
        self.exceeded
    }

    /// Enable the narrow `user -> timestamp -> [m1] -> source -> raw` grammar.
    /// The header/seen-SHA cap includes merges and empty commits. This mode
    /// rejects ambiguous headers and some otherwise valid Git paths.
    pub fn require_source_ids(&mut self, max_headers: usize) {
        self.source = Some(SourceState {
            sha: String::new(),
            seen: HashSet::new(),
            max_headers,
            failed: false,
        });
    }

    /// Sticky failure with no private input embedded in an error message.
    pub fn source_failed(&self) -> bool {
        self.source.as_ref().is_some_and(|s| s.failed)
    }

    fn fail_source(&mut self) {
        if let Some(source) = &mut self.source {
            source.failed = true;
        }
        self.reset();
    }

    /// Feed the next chunk; complete commits are emitted to `sink`.
    pub fn push(&mut self, bytes: &[u8], sink: &mut dyn CommitSink) {
        if self.exceeded || self.source_failed() {
            return;
        }
        let mut offset = self.stats.bytes;
        self.stats.bytes += bytes.len() as u64;

        let mut rest = bytes;
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            if self.exceeded || self.source_failed() {
                return;
            }
            let line = &rest[..nl];
            if self.partial.is_empty() {
                self.line(line, offset, sink);
            } else {
                self.partial.extend_from_slice(line);
                self.flush_partial(sink);
            }
            offset += nl as u64 + 1;
            rest = &rest[nl + 1..];
        }

        if !rest.is_empty() && !self.exceeded && !self.source_failed() {
            if self.partial.is_empty() {
                self.partial_offset = offset;
            }
            if self
                .paths
                .len()
                .saturating_add(self.partial.len() + rest.len())
                > self.max_commit_bytes
            {
                self.overflow();
                return;
            }
            self.partial.extend_from_slice(rest);
        }
    }

    /// Stop parsing and release the pending commit's buffers.
    fn overflow(&mut self) {
        self.exceeded = true;
        self.state = State::Block;
        self.reset();
        self.partial = Vec::new();
        self.paths = String::new();
        self.spans = Vec::new();
        self.unquoted = Vec::new();
        self.files = Vec::new();
    }

    /// Flush the last (possibly unterminated) line and the pending commit.
    pub fn finish(&mut self, sink: &mut dyn CommitSink) {
        if self.exceeded || self.source_failed() {
            return;
        }
        if !self.partial.is_empty() {
            self.flush_partial(sink);
        }

        match self.state {
            // `user:` was the last line: getNextLine fails, the commit is lost.
            State::Timestamp | State::Source => {
                if self.source.is_some() {
                    self.fail_source();
                } else {
                    self.reset();
                }
            }
            State::Meta if self.source.is_some() => self.fail_source(),
            State::Block | State::Meta => self.end_commit(sink),
        }
        self.state = State::Block;
    }

    pub fn stats(&self) -> ParseStats {
        self.stats
    }

    /// Number of `user:` commit headers seen so far, including merge commits
    /// that are dropped for having no files. Comparable with
    /// `git rev-list --count` over the same range.
    pub fn headers_seen(&self) -> u64 {
        self.headers
    }

    /// AI credits dropped because a commit had more than 4 distinct ones.
    pub fn attrs_truncated(&self) -> u64 {
        self.truncated
    }

    fn flush_partial(&mut self, sink: &mut dyn CommitSink) {
        let partial = std::mem::take(&mut self.partial);
        self.line(&partial, self.partial_offset, sink);
        self.partial = partial;
        self.partial.clear();
    }

    fn reset(&mut self) {
        self.username.clear();
        self.timestamp = 0;
        self.paths.clear();
        self.spans.clear();
        self.attrs.clear();
        if let Some(source) = &mut self.source {
            source.sha.clear();
        }
    }

    fn line(&mut self, line: &[u8], offset: u64, sink: &mut dyn CommitSink) {
        // CR is tolerated by ordinary imports, never by the strict source grammar.
        if self.source.is_some() && line.contains(&b'\r') {
            self.fail_source();
            return;
        }
        if self.source.is_some() && line.len() > self.max_commit_bytes {
            self.overflow();
            return;
        }
        // SeekLog::getNextLine strips one trailing carriage return.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        self.stats.lines += 1;

        if self.state == State::Timestamp {
            self.state = State::Meta;
            if self.source.is_some() {
                let valid = std::str::from_utf8(line).ok().and_then(|s| {
                    let digits = s.strip_prefix('-').unwrap_or(s);
                    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| s.parse::<i64>().ok())
                        .flatten()
                });
                if let Some(time) = valid.filter(|&t| t != 0) {
                    self.timestamp = time;
                } else {
                    self.fail_source();
                }
                return;
            }
            self.timestamp = atol(line);
            // "this isnt a commit we are parsing, abort"
            if self.timestamp == 0 {
                self.state = State::Block;
                self.stats.skipped_lines += 1;
                self.reset();
            }
            return;
        }

        if self.state == State::Meta {
            self.state = if self.source.is_some() {
                State::Source
            } else {
                State::Block
            };
            if let Some(meta) = line.strip_prefix(b"m1:") {
                self.meta_line(meta);
                return;
            }
        }

        if self.state == State::Source {
            let Some(sha) = line
                .strip_prefix(b"source:")
                .filter(|s| crate::source::valid_sha(s))
            else {
                self.fail_source();
                return;
            };
            let source = self
                .source
                .as_mut()
                .expect("source state requires source mode");
            let sha = std::str::from_utf8(sha).expect("SHA grammar is ASCII");
            if source.seen.len() >= source.max_headers || !source.seen.insert(sha.to_owned()) {
                self.fail_source();
                return;
            }
            source.sha.push_str(sha);
            self.state = State::Block;
            return;
        }

        if line.is_empty() {
            self.end_commit(sink);
            return;
        }

        if let Some(name) = line.strip_prefix(b"user:") {
            if self.source.is_some() {
                // The ordinary parser intentionally keeps old file spans when
                // another user appears. Trusted source receipts cannot do that.
                if !self.spans.is_empty()
                    || name.is_empty()
                    || name.iter().any(|b| *b < 32 || *b == 127)
                    || self.headers >= self.source.as_ref().unwrap().max_headers as u64
                {
                    self.fail_source();
                    return;
                }
                self.reset();
            }
            self.headers += 1;
            if self.spans.is_empty() {
                self.commit_offset = offset;
            }
            self.username.clear();
            filter_utf8_into(name, &mut self.username);
            self.attrs.clear();
            self.state = State::Timestamp;
            return;
        }

        // A meta line out of place (not right after a timestamp) is never a
        // file line, even when its subject holds a TAB.
        if line.starts_with(b"m1:") {
            if self.source.is_some() {
                self.fail_source();
                return;
            }
            self.stats.skipped_lines += 1;
            return;
        }

        // "should see username before files": parseCommit returns false and
        // the partial commit is discarded.
        if self.username.is_empty() {
            if self.source.is_some() {
                self.fail_source();
                return;
            }
            self.stats.skipped_lines += 1;
            self.reset();
            return;
        }

        if self.source.is_some() && !valid_source_raw(line) {
            self.fail_source();
            return;
        }
        let Some(tab) = line.iter().position(|&b| b == b'\t') else {
            self.stats.skipped_lines += 1;
            return;
        };
        if tab == 0 || tab == line.len() - 1 {
            self.stats.skipped_lines += 1;
            return;
        }

        // Gource keeps the raw status letter; processCommit treats "A" as a
        // create, "D" as a remove and every other letter (M, T, U, X, ...)
        // as a modification.
        let kind = match line[tab - 1] {
            b'A' => ChangeKind::Add,
            b'D' => ChangeKind::Delete,
            _ => ChangeKind::Modify,
        };

        let mut file = &line[tab + 1..];
        if self.source.is_some() && !valid_source_quoting(file) {
            self.fail_source();
            return;
        }
        if file.first() == Some(&b'"') && file.last() == Some(&b'"') {
            if file.len() <= 2 {
                self.stats.skipped_lines += 1;
                return;
            }
            unquote_c_style(&file[1..file.len() - 1], &mut self.unquoted);
            file = &self.unquoted;
        }

        if self.source.is_some() && !crate::source::safe_path_bytes(file) {
            self.fail_source();
            return;
        }

        if self.spans.len() >= self.max_commit_files {
            self.overflow();
            return;
        }
        let start = self.paths.len();
        filter_utf8_into(file, &mut self.paths);
        // RCommitFile prepends a root slash.
        if self.paths.as_bytes().get(start) != Some(&b'/') {
            self.paths.insert(start, '/');
        }
        if self.paths.len() > self.max_commit_bytes {
            self.overflow();
            return;
        }
        self.spans.push((start, self.paths.len(), kind));
    }

    fn end_commit(&mut self, sink: &mut dyn CommitSink) {
        if !self.username.is_empty() && !self.spans.is_empty() {
            let mut files = recycle(std::mem::take(&mut self.files));
            files.extend(self.spans.iter().map(|&(start, end, kind)| ParsedFile {
                path: &self.paths[start..end],
                kind,
                colour: None,
                event: ChangeEvent::Commit,
            }));

            let mut attrs = attrs::recycle(std::mem::take(&mut self.attr_views));
            self.attrs.extend_into(&mut attrs);
            if !self.cap.admit(&attrs) {
                attrs.clear();
                self.stats.bad_meta += 1;
            }

            self.stats.commits += 1;
            self.stats.files += files.len() as u64;

            let commit = ParsedCommit {
                timestamp: self.timestamp,
                username: &self.username,
                files: &files,
                attrs: &attrs,
                byte_offset: self.commit_offset,
            };
            if let Some(source) = &self.source {
                sink.source_commit(&commit, &source.sha);
            } else {
                sink.commit(&commit);
            }

            self.files = recycle(files);
            self.attr_views = attrs::recycle(attrs);
        }
        self.reset();
    }

    /// The `m1:` line of the pending commit (without the prefix).
    fn meta_line(&mut self, meta: &[u8]) {
        self.attrs.clear();
        if !self.classify_meta(meta) {
            self.stats.bad_meta += 1;
            if self.source.is_some() {
                self.fail_source();
            }
        }
    }

    /// Classify one meta line into `self.attrs`; false when it is malformed.
    fn classify_meta(&mut self, meta: &[u8]) -> bool {
        let Some(rs) = meta.iter().position(|&b| b == 0x1e) else {
            return false;
        };
        let mut pieces = meta[..rs].split(|&b| b == 0x1f);
        let (Some(an), Some(ae), Some(cn), Some(ce)) =
            (pieces.next(), pieces.next(), pieces.next(), pieces.next())
        else {
            return false;
        };
        // The trailer slot is always there, empty without trailers.
        let mut pieces = pieces.peekable();
        if pieces.peek().is_none() {
            return false;
        }
        let control = |s: &[u8]| s.iter().any(|&b| b < 0x20);
        let space = |s: &[u8]| s.iter().any(|b| b.is_ascii_whitespace());
        if [an, ae, cn, ce].into_iter().any(control) || space(ae) || space(ce) {
            return false;
        }

        let mut trailers = recycle_trailers(std::mem::take(&mut self.trailers));
        for piece in pieces {
            if piece.is_empty() {
                continue;
            }
            let Some(colon) = piece.iter().position(|&b| b == b':') else {
                self.trailers = recycle_trailers(trailers);
                return false;
            };
            // Non-UTF-8 text cannot name an agent (the registry is ASCII).
            if let (Ok(key), Ok(value)) = (
                std::str::from_utf8(&piece[..colon]),
                std::str::from_utf8(&piece[colon + 1..]),
            ) {
                trailers.push(Trailer { key, value });
            }
        }

        let text = |s| std::str::from_utf8(s).unwrap_or("");
        let signals = CommitSignals {
            author: Identity {
                name: text(an),
                email: text(ae),
            },
            committer: Identity {
                name: text(cn),
                email: text(ce),
            },
            subject: "",
            trailers: &trailers,
        };
        let mut buf = std::mem::take(&mut self.attr_buf).recycle();
        classify(&signals, &mut buf);
        self.truncated += buf.truncated() as u64;

        // An agent identity that the mailmap renamed into a person.
        self.raw_name.clear();
        filter_utf8_into(an, &mut self.raw_name);
        if self.raw_name != self.username {
            buf.downgrade_agents();
        }
        for a in buf.as_slice() {
            self.attrs.push(a.kind, a.agent, a.model);
        }

        self.attr_buf = buf.recycle();
        self.trailers = recycle_trailers(trailers);
        true
    }
}

/// Reuse a vector's allocation for a different lifetime. The in-place
/// `collect` specialisation keeps the buffer because the element layout is
/// identical.
fn recycle<'b>(mut v: Vec<ParsedFile<'_>>) -> Vec<ParsedFile<'b>> {
    v.clear();
    v.into_iter().map(|_| unreachable!()).collect()
}

fn recycle_trailers<'b>(mut v: Vec<Trailer<'_>>) -> Vec<Trailer<'b>> {
    v.clear();
    v.into_iter().map(|_| unreachable!()).collect()
}

/// Source producers use a single-path `--raw --no-renames` record, never a
/// generic line containing a TAB or a rename/copy/combined-diff record.
fn valid_source_raw(line: &[u8]) -> bool {
    let Some(tab) = line.iter().position(|&b| b == b'\t') else {
        return false;
    };
    if tab + 1 == line.len() || line[tab + 1..].contains(&b'\t') {
        return false;
    }
    let Some(header) = line[..tab].strip_prefix(b":") else {
        return false;
    };
    let mut fields = header.split(|&b| b == b' ');
    let (Some(old_mode), Some(new_mode), Some(old_oid), Some(new_oid), Some(action)) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return false;
    };
    let mode = |s: &[u8]| s.len() == 6 && s.iter().all(|b| (b'0'..=b'7').contains(b));
    let oid = |s: &[u8]| {
        (4..=64).contains(&s.len())
            && s.iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    };
    fields.next().is_none()
        && mode(old_mode)
        && mode(new_mode)
        && oid(old_oid)
        && oid(new_oid)
        && matches!(action, b"A" | b"D" | b"M" | b"T" | b"U" | b"X")
}

/// C `atol`: optional leading whitespace and sign, then decimal digits up to
/// the first non-digit. Saturates instead of overflowing.
fn atol(s: &[u8]) -> i64 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut value: i64 = 0;
    while let Some(&d) = s.get(i) {
        if !d.is_ascii_digit() {
            break;
        }
        let digit = i64::from(d - b'0');
        value = if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        };
        i += 1;
    }
    value
}

/// Source receipts only accept complete Git C-style quoting. Keep the ordinary
/// parser's permissive decoding unchanged, including its unknown escapes.
fn valid_source_quoting(path: &[u8]) -> bool {
    if path.first() != Some(&b'"') {
        return !path.contains(&b'"');
    }
    if path.len() <= 2 || path.last() != Some(&b'"') {
        return false;
    }
    let body = &path[1..path.len() - 1];
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            b'"' => return false,
            b'\\' => {
                let Some(&escape) = body.get(i + 1) else {
                    return false;
                };
                if matches!(
                    escape,
                    b'a' | b'b' | b't' | b'n' | b'v' | b'f' | b'r' | b'"' | b'\\'
                ) {
                    i += 2;
                } else if (b'0'..=b'3').contains(&escape)
                    && body.get(i + 2).is_some_and(|b| (b'0'..=b'7').contains(b))
                    && body.get(i + 3).is_some_and(|b| (b'0'..=b'7').contains(b))
                {
                    i += 4;
                } else {
                    return false;
                }
            }
            _ => i += 1,
        }
    }
    true
}

/// Decode the body of a C-style quoted path (`quote_c_style` in git):
/// `\a \b \t \n \v \f \r \" \\` and three-digit octal escapes. Unknown
/// escapes are kept verbatim.
fn unquote_c_style(src: &[u8], out: &mut Vec<u8>) {
    out.clear();
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b != b'\\' || i + 1 == src.len() {
            out.push(b);
            i += 1;
            continue;
        }
        let e = src[i + 1];
        let simple = match e {
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b't' => Some(b'\t'),
            b'n' => Some(b'\n'),
            b'v' => Some(0x0b),
            b'f' => Some(0x0c),
            b'r' => Some(b'\r'),
            b'"' => Some(b'"'),
            b'\\' => Some(b'\\'),
            _ => None,
        };
        if let Some(c) = simple {
            out.push(c);
            i += 2;
            continue;
        }
        let octal = |b: u8| (b'0'..=b'7').contains(&b);
        if (b'0'..=b'3').contains(&e) && i + 3 < src.len() && octal(src[i + 2]) && octal(src[i + 3])
        {
            out.push(((e - b'0') << 6) | ((src[i + 2] - b'0') << 3) | (src[i + 3] - b'0'));
            i += 4;
            continue;
        }
        out.push(b'\\');
        out.push(e);
        i += 2;
    }
}

/// Append `src` to `out`, replacing invalid UTF-8 the way
/// `RCommitLog::filter_utf8` does (`utf8::replace_invalid(..., '?')` from
/// utf8-cpp): one `?` per invalid lead byte, one `?` per bad sequence
/// (skipping its trail bytes), and a single `?` for a sequence truncated by
/// the end of the string.
fn filter_utf8_into(src: &[u8], out: &mut String) {
    if let Ok(s) = std::str::from_utf8(src) {
        out.push_str(s);
        return;
    }

    let is_trail = |b: u8| b & 0xc0 == 0x80;
    let mut i = 0;
    while i < src.len() {
        let lead = src[i];
        let len = match lead {
            0x00..=0x7f => {
                out.push(char::from(lead));
                i += 1;
                continue;
            }
            _ if lead >> 5 == 0x6 => 2,
            _ if lead >> 4 == 0xe => 3,
            _ if lead >> 3 == 0x1e => 4,
            _ => {
                // INVALID_LEAD
                out.push('?');
                i += 1;
                continue;
            }
        };

        let mut cp = u32::from(lead) & (0xff >> (len + 1));
        let mut bad = false;
        for j in 1..len {
            match src.get(i + j) {
                // NOT_ENOUGH_ROOM: one mark, and the rest of the input is gone.
                None => {
                    out.push('?');
                    return;
                }
                Some(&b) if is_trail(b) => cp = (cp << 6) | u32::from(b & 0x3f),
                Some(_) => {
                    bad = true;
                    break;
                }
            }
        }

        if !bad {
            let overlong = if cp < 0x80 {
                len != 1
            } else if cp < 0x800 {
                len != 2
            } else if cp < 0x10000 {
                len != 3
            } else {
                false
            };
            match char::from_u32(cp) {
                Some(c) if !overlong => {
                    out.push(c);
                    i += len;
                    continue;
                }
                _ => bad = true,
            }
        }

        debug_assert!(bad);
        // INCOMPLETE_SEQUENCE / OVERLONG_SEQUENCE / INVALID_CODE_POINT
        out.push('?');
        i += 1;
        while i < src.len() && is_trail(src[i]) {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(b: &[u8]) -> String {
        let mut s = String::new();
        filter_utf8_into(b, &mut s);
        s
    }

    #[test]
    fn filter_utf8_matches_utf8_cpp() {
        assert_eq!(filter(b"plain"), "plain");
        assert_eq!(filter("ü/ñ".as_bytes()), "ü/ñ");
        // stray trail bytes: INVALID_LEAD, one '?' each
        assert_eq!(filter(b"a\x80\x80b"), "a??b");
        assert_eq!(filter(b"a\xffb"), "a?b");
        // incomplete sequence followed by ASCII
        assert_eq!(filter(b"a\xc3b"), "a?b");
        assert_eq!(filter(b"\xe4\xb8a"), "?a");
        // truncated at the end: NOT_ENOUGH_ROOM swallows the rest
        assert_eq!(filter(b"ab\xe4\xb8"), "ab?");
        // overlong encodings
        assert_eq!(filter(b"\xc0\xafx"), "?x");
        assert_eq!(filter(b"\xe0\x80\xafx"), "?x");
        assert_eq!(filter(b"\xf0\x80\x80\xafx"), "?x");
        // surrogate, and beyond U+10FFFF
        assert_eq!(filter(b"\xed\xa0\x80x"), "?x");
        assert_eq!(filter(b"\xf4\x90\x80\x80x"), "?x");
        // valid sequences around an invalid one
        assert_eq!(filter(b"\xc3\xbc\xff\xf0\x9f\x98\x80"), "ü?😀");
    }

    #[test]
    fn atol_semantics() {
        assert_eq!(atol(b"1234"), 1234);
        assert_eq!(atol(b"  -12x"), -12);
        assert_eq!(atol(b"+7"), 7);
        assert_eq!(atol(b"abc"), 0);
        assert_eq!(atol(b""), 0);
        assert_eq!(atol(b"99999999999999999999999"), i64::MAX);
    }

    #[test]
    fn unquote() {
        let mut out = Vec::new();
        unquote_c_style(br#"tab\tq\n\"\\\303\274\x"#, &mut out);
        assert_eq!(out, b"tab\tq\n\"\\\xc3\xbc\\x");
        unquote_c_style(br"end\", &mut out);
        assert_eq!(out, b"end\\");
        unquote_c_style(br"\7", &mut out);
        assert_eq!(out, b"\\7");
        unquote_c_style(br"\a\b\v\f\r\101", &mut out);
        assert_eq!(out, b"\x07\x08\x0b\x0c\rA");
    }

    #[test]
    fn recycle_keeps_allocation() {
        let mut v: Vec<ParsedFile<'static>> = Vec::with_capacity(16);
        let ptr = v.as_ptr() as usize;
        v.push(ParsedFile {
            path: "/a",
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        });
        let w: Vec<ParsedFile<'_>> = recycle(v);
        assert_eq!(w.as_ptr() as usize, ptr);
        assert!(w.capacity() >= 16);
    }
}
