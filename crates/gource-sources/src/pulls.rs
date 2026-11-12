// SPDX-License-Identifier: GPL-3.0-or-later

//! Pull request events in the commit stream (CLAUDE.md: "model 'issue
//! opened', 'PR merged', etc. as new event kinds in the same stream as
//! commits").
//!
//! Each pull request is a node of the tree, `/pull requests/#123 Title`:
//! its author adds it when the pull request is opened, and it is deleted
//! when the pull request is merged or closed, with the event kind in the
//! change ([`ChangeEvent`]) so the engine colours the beam. The colour
//! column holds the state's colour, so C++ Gource (which ignores the meta
//! field) still shows the nodes:
//!
//! ```text
//! 1696204800|alice|A|/pull requests/#4821 Fix the seek bar|3FB950|ev=pr-opened
//! 1696291200|alice|D|/pull requests/#4821 Fix the seek bar|A371F7|ev=pr-merged
//! ```
//!
//! [`events`] turns records (as a provider fetched them; this crate does no
//! I/O) into time-ordered events, and [`PullRequestMerger`] inserts them
//! into a commit stream before the first commit that is later than them.
//! A merge or close is credited to the pull request's author: the list
//! endpoint does not name who merged.

use gource_core::events::{ChangeEvent, ChangeKind};

use crate::{CommitSink, ParsedCommit, ParsedFile};

/// The directory pull request nodes live in.
pub const DIR: &str = "/pull requests/";
/// Colour of an open pull request's node.
pub const OPEN_COLOUR: [u8; 3] = [0x3F, 0xB9, 0x50];
/// Colour of a merged pull request (also its beam, `PR_MERGED_COLOUR`).
pub const MERGED_COLOUR: [u8; 3] = [0xA3, 0x71, 0xF7];
/// Colour of a pull request closed without merging.
pub const CLOSED_COLOUR: [u8; 3] = [0x8B, 0x94, 0x9E];
/// Longest title kept in a node's name, in characters.
pub const MAX_TITLE_CHARS: usize = 60;

/// One pull request, as a provider read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    /// Login of the author.
    pub author: String,
    /// Unix seconds.
    pub created_at: i64,
    pub merged_at: Option<i64>,
    /// Set for merged pull requests too (GitHub closes on merge).
    pub closed_at: Option<i64>,
}

/// One event: a pull request opened, merged or closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrEvent {
    pub timestamp: i64,
    pub author: String,
    pub path: String,
    pub kind: ChangeKind,
    pub colour: [u8; 3],
    pub event: ChangeEvent,
}

/// The node of pull request `number`: `/pull requests/#123 Title`. Slashes
/// in the title become `-` (they would make directories), control
/// characters are dropped, and long titles are cut at
/// [`MAX_TITLE_CHARS`] characters with `…`.
pub fn node_path(number: u64, title: &str) -> String {
    let mut name = String::new();
    let mut chars = 0;
    let mut cut = false;
    for c in title.trim().chars() {
        if c.is_control() {
            continue;
        }
        if chars == MAX_TITLE_CHARS {
            cut = true;
            break;
        }
        name.push(if matches!(c, '/' | '\\') { '-' } else { c });
        chars += 1;
    }
    let name = name.trim_end();
    let mut path = format!("{DIR}#{number}");
    if !name.is_empty() {
        path.push(' ');
        path.push_str(name);
        if cut {
            path.push('…');
        }
    }
    path
}

/// The events of `prs` within `[since, until]` (unix seconds, inclusive;
/// `None` = open), ordered by time, then number, an open before its end.
/// A pull request opened before `since` and merged inside the range is
/// opened at `since`, so its node exists when it is merged.
pub fn events(prs: &[PullRequest], since: Option<i64>, until: Option<i64>) -> Vec<PrEvent> {
    let in_range = |t: i64| since.is_none_or(|s| t >= s) && until.is_none_or(|u| t <= u);
    let mut out = Vec::with_capacity(prs.len() * 2);
    for pr in prs {
        let path = node_path(pr.number, &pr.title);
        let end = match (pr.merged_at, pr.closed_at) {
            (Some(t), _) => Some((t, ChangeEvent::PrMerged, MERGED_COLOUR)),
            (None, Some(t)) => Some((t, ChangeEvent::PrClosed, CLOSED_COLOUR)),
            (None, None) => None,
        };
        let end = end.filter(|&(t, ..)| in_range(t) && t >= pr.created_at);
        let opened = if in_range(pr.created_at) {
            Some(pr.created_at)
        } else if let (Some(s), Some(_)) = (since, end) {
            // Opened before the range, ended inside it.
            (pr.created_at < s).then_some(s)
        } else {
            None
        };
        let Some(opened) = opened else {
            continue;
        };
        out.push(PrEvent {
            timestamp: opened,
            author: pr.author.clone(),
            path: path.clone(),
            kind: ChangeKind::Add,
            colour: OPEN_COLOUR,
            event: ChangeEvent::PrOpened,
        });
        if let Some((t, event, colour)) = end {
            out.push(PrEvent {
                timestamp: t.max(opened),
                author: pr.author.clone(),
                path,
                kind: ChangeKind::Delete,
                colour,
                event,
            });
        }
    }
    // Stable: for one number, the open stays before its end.
    out.sort_by_key(|e| (e.timestamp, number_of(&e.path)));
    out
}

/// A line of a pull request list that could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineError {
    /// 1-based line number.
    pub line: usize,
    pub message: &'static str,
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for LineError {}

/// Reads pull requests in the line format of `gource-rs convert --pulls`:
/// one per line, six tab-separated fields
/// `number created merged closed author title`, times in unix seconds or
/// `-` for none. The title is the rest of the line, so it may hold tabs.
/// Empty lines are skipped and a trailing `\r` is ignored.
pub fn parse_lines(text: &str) -> Result<Vec<PullRequest>, LineError> {
    let mut out = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        let err = |message| LineError {
            line: i + 1,
            message,
        };
        let time = |s: &str| -> Result<Option<i64>, LineError> {
            if s == "-" {
                return Ok(None);
            }
            s.parse::<i64>()
                .map(Some)
                .map_err(|_| err("time is not unix seconds or '-'"))
        };
        let mut f = line.splitn(6, '\t');
        let (Some(number), Some(created), Some(merged), Some(closed), Some(author), Some(title)) =
            (f.next(), f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            return Err(err("expected 6 tab-separated fields"));
        };
        let number = number
            .parse::<u64>()
            .map_err(|_| err("number is not an integer"))?;
        let created_at = time(created)?.ok_or_else(|| err("created time is required"))?;
        if author.is_empty() {
            return Err(err("author is empty"));
        }
        out.push(PullRequest {
            number,
            title: title.to_string(),
            author: author.to_string(),
            created_at,
            merged_at: time(merged)?,
            closed_at: time(closed)?,
        });
    }
    Ok(out)
}

fn number_of(path: &str) -> u64 {
    path[DIR.len() + 1..]
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// A [`CommitSink`] that passes commits on to its inner sink and inserts
/// the pull request events before the first commit later than them; the
/// rest follow at [`PullRequestMerger::finish`]. Each event is a commit of
/// its own. With no events it only passes commits through.
pub struct PullRequestMerger<S> {
    inner: S,
    events: Vec<PrEvent>,
    next: usize,
    last_offset: u64,
}

impl<S: CommitSink> PullRequestMerger<S> {
    /// `events` must be ordered by time ([`events`] returns them so).
    pub fn new(inner: S, events: Vec<PrEvent>) -> Self {
        PullRequestMerger {
            inner,
            events,
            next: 0,
            last_offset: 0,
        }
    }

    /// Emit the events not emitted yet.
    pub fn finish(&mut self) {
        self.emit_until(i64::MAX);
    }

    /// Events emitted so far.
    pub fn emitted(&self) -> usize {
        self.next
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    fn emit_until(&mut self, timestamp: i64) {
        while let Some(e) = self.events.get(self.next) {
            if e.timestamp > timestamp {
                break;
            }
            let files = [ParsedFile {
                path: &e.path,
                kind: e.kind,
                colour: Some(e.colour),
                event: e.event,
            }];
            self.inner.commit(&ParsedCommit {
                timestamp: e.timestamp,
                username: &e.author,
                files: &files,
                attrs: &[],
                byte_offset: self.last_offset,
            });
            self.next += 1;
        }
    }
}

impl<S: CommitSink> CommitSink for PullRequestMerger<S> {
    fn source_commit(&mut self, commit: &ParsedCommit<'_>, sha: &str) {
        if let Some(before) = commit.timestamp.checked_sub(1) {
            self.emit_until(before);
        }
        self.last_offset = commit.byte_offset;
        self.inner.source_commit(commit, sha);
    }

    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        // Events strictly earlier than the commit go first; a commit and an
        // event of the same second keep the commit first, including i64::MIN.
        if let Some(before) = commit.timestamp.checked_sub(1) {
            self.emit_until(before);
        }
        self.last_offset = commit.byte_offset;
        self.inner.commit(commit);
    }

    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[crate::ParsedPrChange<'_>]) {
        if let Some(before) = commit.timestamp.checked_sub(1) {
            self.emit_until(before);
        }
        self.last_offset = commit.byte_offset;
        self.inner.commit_pr(commit, records);
    }

    fn pr_log(&mut self, update: gource_core::events::PrLogUpdate) {
        self.inner.pr_log(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CustomLogWriter;

    fn pr(
        number: u64,
        title: &str,
        created: i64,
        merged: Option<i64>,
        closed: Option<i64>,
    ) -> PullRequest {
        PullRequest {
            number,
            title: title.into(),
            author: format!("dev{number}"),
            created_at: created,
            merged_at: merged,
            closed_at: closed,
        }
    }

    #[test]
    fn node_names() {
        assert_eq!(
            node_path(7, "Fix the seek bar"),
            "/pull requests/#7 Fix the seek bar"
        );
        assert_eq!(node_path(8, " a/b\\c \n"), "/pull requests/#8 a-b-c");
        assert_eq!(node_path(9, ""), "/pull requests/#9");
        assert_eq!(node_path(10, "\u{1}\u{7}"), "/pull requests/#10");
        let long = "x".repeat(80);
        let p = node_path(11, &long);
        assert_eq!(p, format!("/pull requests/#11 {}…", "x".repeat(60)));
        assert_eq!(
            node_path(12, &"é".repeat(60)),
            format!("/pull requests/#12 {}", "é".repeat(60))
        );
    }

    #[test]
    fn events_are_ordered_and_coloured() {
        let prs = [
            pr(2, "Second", 200, Some(400), Some(400)),
            pr(1, "First", 100, None, Some(300)),
            pr(3, "Open", 250, None, None),
        ];
        let ev = events(&prs, None, None);
        let summary: Vec<_> = ev
            .iter()
            .map(|e| (e.timestamp, e.path.as_str(), e.kind, e.event))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    100,
                    "/pull requests/#1 First",
                    ChangeKind::Add,
                    ChangeEvent::PrOpened
                ),
                (
                    200,
                    "/pull requests/#2 Second",
                    ChangeKind::Add,
                    ChangeEvent::PrOpened
                ),
                (
                    250,
                    "/pull requests/#3 Open",
                    ChangeKind::Add,
                    ChangeEvent::PrOpened
                ),
                (
                    300,
                    "/pull requests/#1 First",
                    ChangeKind::Delete,
                    ChangeEvent::PrClosed
                ),
                (
                    400,
                    "/pull requests/#2 Second",
                    ChangeKind::Delete,
                    ChangeEvent::PrMerged
                ),
            ]
        );
        assert_eq!(ev[3].colour, CLOSED_COLOUR);
        assert_eq!(ev[4].colour, MERGED_COLOUR);
        assert_eq!(ev[4].author, "dev2");
    }

    #[test]
    fn the_date_range_applies_and_keeps_nodes_consistent() {
        let prs = [
            pr(1, "Before, merged inside", 50, Some(150), Some(150)),
            pr(2, "Inside, merged after", 120, Some(500), Some(500)),
            pr(3, "Entirely before", 10, Some(20), Some(20)),
            pr(4, "Opened before, still open", 30, None, None),
            pr(5, "Same second", 130, Some(130), Some(130)),
        ];
        let ev = events(&prs, Some(100), Some(200));
        let summary: Vec<_> = ev
            .iter()
            .map(|e| (e.timestamp, number_of(&e.path), e.event))
            .collect();
        assert_eq!(
            summary,
            [
                (100, 1, ChangeEvent::PrOpened),
                (120, 2, ChangeEvent::PrOpened),
                (130, 5, ChangeEvent::PrOpened),
                (130, 5, ChangeEvent::PrMerged),
                (150, 1, ChangeEvent::PrMerged),
            ]
        );
    }

    struct Collect(Vec<(i64, String, String)>);
    impl CommitSink for Collect {
        fn commit(&mut self, c: &ParsedCommit<'_>) {
            for f in c.files {
                self.0
                    .push((c.timestamp, c.username.to_string(), f.path.to_string()));
            }
        }
    }

    #[test]
    fn the_merger_interleaves_by_time() {
        let ev = events(&[pr(1, "One", 150, Some(300), Some(300))], None, None);
        let mut m = PullRequestMerger::new(Collect(Vec::new()), ev);
        for (t, path) in [(100, "/a"), (150, "/b"), (200, "/c")] {
            let files = [ParsedFile {
                path,
                kind: ChangeKind::Add,
                colour: None,
                event: ChangeEvent::Commit,
            }];
            m.commit(&ParsedCommit {
                timestamp: t,
                username: "jane",
                files: &files,
                attrs: &[],
                byte_offset: t as u64,
            });
        }
        assert_eq!(m.emitted(), 1);
        m.finish();
        assert_eq!(m.emitted(), 2);
        let out = m.into_inner();
        let order: Vec<_> = out.0.iter().map(|(t, _, p)| (*t, p.as_str())).collect();
        assert_eq!(
            order,
            [
                (100, "/a"),
                (150, "/b"),
                (150, "/pull requests/#1 One"),
                (200, "/c"),
                (300, "/pull requests/#1 One"),
            ]
        );
    }

    #[test]
    fn written_lines_carry_the_colour_and_the_event() {
        let ev = events(
            &[pr(
                4821,
                "Fix the seek bar",
                1_696_204_800,
                Some(1_696_291_200),
                Some(1_696_291_200),
            )],
            None,
            None,
        );
        let mut m = PullRequestMerger::new(CustomLogWriter::new(Vec::new()), ev);
        m.finish();
        let text = String::from_utf8(m.into_inner().finish().unwrap()).unwrap();
        assert_eq!(
            text,
            "1696204800|dev4821|A|/pull requests/#4821 Fix the seek bar|3FB950|ev=pr-opened\n\
             1696291200|dev4821|D|/pull requests/#4821 Fix the seek bar|A371F7|ev=pr-merged\n"
        );
    }

    #[test]
    fn parses_every_field() {
        let prs =
            parse_lines("4821\t1696204800\t1696291200\t1696291200\talice\tFix the seek bar\n")
                .unwrap();
        assert_eq!(
            prs,
            [PullRequest {
                number: 4821,
                title: "Fix the seek bar".into(),
                author: "alice".into(),
                created_at: 1_696_204_800,
                merged_at: Some(1_696_291_200),
                closed_at: Some(1_696_291_200),
            }]
        );
    }

    #[test]
    fn dash_means_none_and_blank_lines_and_crlf_are_ignored() {
        let prs =
            parse_lines("\n7\t10\t-\t-\tbob\topen one\r\n\n8\t20\t-\t30\tcy\tclosed\n").unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!((prs[0].merged_at, prs[0].closed_at), (None, None));
        assert_eq!(prs[0].title, "open one");
        assert_eq!((prs[1].merged_at, prs[1].closed_at), (None, Some(30)));
    }

    #[test]
    fn titles_keep_tabs_and_lose_nothing_else() {
        let prs = parse_lines("1\t10\t-\t-\tann\ta\tb\t\tc").unwrap();
        assert_eq!(prs[0].author, "ann");
        assert_eq!(prs[0].title, "a\tb\t\tc");
        // The node name drops the control characters, as for any title.
        assert_eq!(node_path(1, &prs[0].title), "/pull requests/#1 abc");
    }

    #[test]
    fn errors_name_the_line() {
        let e = parse_lines("1\t10\t-\t-\tann\tok\nx\t10\t-\t-\tann\tbad\n").unwrap_err();
        assert_eq!(e.to_string(), "line 2: number is not an integer");
        let e = parse_lines("1\t10\t-\t-\tann").unwrap_err();
        assert_eq!(e.to_string(), "line 1: expected 6 tab-separated fields");
        let e = parse_lines("1\t-\t-\t-\tann\tt").unwrap_err();
        assert_eq!(e.to_string(), "line 1: created time is required");
        let e = parse_lines("1\t10\tsoon\t-\tann\tt").unwrap_err();
        assert_eq!(e.to_string(), "line 1: time is not unix seconds or '-'");
        let e = parse_lines("1\t10\t-\t-\t\tt").unwrap_err();
        assert_eq!(e.to_string(), "line 1: author is empty");
    }
}
