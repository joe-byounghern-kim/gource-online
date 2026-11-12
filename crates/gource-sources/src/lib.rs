// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Input adapters for the Gource port.
//!
//! Each adapter converts an external representation into the event stream
//! consumed by `gource-core`. The pipe-delimited custom log
//! (`timestamp|username|A/M/D|/path[|RRGGBB][|meta]`) is the baseline
//! interchange format; further adapters (git `--raw` output, pull request
//! events in [`pulls`]) feed the same stream. Adapters take bytes or strings, never spawn
//! processes, so they work in the browser as well as natively.
//!
//! Parsers are push-based and streaming: feed arbitrary byte chunks with
//! `push`, then call `finish`. Every parsed commit goes to a [`CommitSink`]:
//!
//! - [`EventLogSink`] appends straight into a [`gource_core::events::EventLog`]
//!   (native runs, tests).
//! - [`BatchEncoder`] interns strings and encodes compact binary batches that
//!   a browser Web Worker posts to the main thread, where [`apply_batch`]
//!   appends them to the main thread's `EventLog` with identical string ids.
//! - [`CustomLogWriter`] writes the custom log format (the clone service uses
//!   it to turn `git log --raw` output into the interchange format).
//!
//! AI attributions travel with the commits ([`ParsedCommit::attrs`]): the
//! custom log carries them in its optional meta field ([`meta`]), and the
//! git parser derives them from identities and trailers with the shared
//! classifier ([`classify`]). Every producer hands out sets in the canonical
//! order of [`canonicalize`] and stops attributing after
//! [`gource_core::events::MAX_ATTR_SETS`] distinct sets, so all sinks of
//! one stream number the same sets.

use gource_core::events::{
    AttrKind, ChangeEvent, ChangeKind, EventLog, PrAssociation, PrLifecycle, PrLogUpdate,
    PrPayload, PrState, StrId, is_pr_label,
};

mod attrs;
mod batch;
mod bounded;
pub mod classify;
mod custom;
mod datetime;
mod git;
pub mod meta;
pub mod pulls;
pub mod rich_pulls;
mod sink;
pub mod source;
mod utf8filter;
mod writer;

pub use attrs::canonicalize;
pub use batch::{BatchEncoder, BatchError, BatchSummary, apply_batch, encode_event_log};
pub use bounded::{BoundedWriter, CountingWriter};
pub use custom::CustomLogParser;
pub use git::GitRawParser;
pub use sink::EventLogSink;
pub use writer::{CustomLogWriter, WriteStats};

/// One file entry of a parsed commit.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParsedFile<'a> {
    /// Absolute path, rooted with `/` (as `RCommitFile` does).
    pub path: &'a str,
    pub kind: ChangeKind,
    pub colour: Option<[u8; 3]>,
    /// [`ChangeEvent::Commit`] for a commit's change; pull request events
    /// otherwise (the custom log's `ev` meta value).
    pub event: ChangeEvent,
}

/// One AI credit of a parsed commit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParsedAttribution<'a> {
    pub kind: AttrKind,
    /// Agent id, `[a-z0-9][a-z0-9-]{0,31}`.
    pub agent: &'a str,
    /// Model text, at most 64 bytes of `[A-Za-z0-9 ._:/+,-]`.
    pub model: Option<&'a str>,
}

/// One parsed commit, borrowed from the parser's buffers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommit<'a> {
    pub timestamp: i64,
    pub username: &'a str,
    pub files: &'a [ParsedFile<'a>],
    /// AI credits in canonical order ([`canonicalize`]), at most 4; empty
    /// for most commits.
    pub attrs: &'a [ParsedAttribution<'a>],
    /// Byte offset of the commit's first line in the input.
    pub byte_offset: u64,
}

/// Borrowed lifecycle fields of one rich change, separate from its actor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParsedPrLifecycle<'a> {
    pub number: u64,
    pub owner: &'a str,
    pub target: Option<&'a str>,
    pub created: i64,
    pub state: PrState,
    pub initial: bool,
}

/// Only trusted producers can establish the exact repository/SHA/node binding.
/// The custom format's bare `pr` number cannot recreate this association.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParsedPrAssociation<'a> {
    pub repository: &'a str,
    pub number: u64,
    pub source_sha: &'a str,
    pub pr_node: &'a str,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParsedPrPayload<'a> {
    Lifecycle(ParsedPrLifecycle<'a>),
    Association(ParsedPrAssociation<'a>),
}

impl<'a> ParsedPrPayload<'a> {
    pub fn from_log(log: &'a EventLog, payload: PrPayload) -> Self {
        match payload {
            PrPayload::Lifecycle(p) => Self::Lifecycle(ParsedPrLifecycle {
                number: p.number,
                owner: log.str(p.owner),
                target: p.target.map(|id| log.str(id)),
                created: p.created,
                state: p.state,
                initial: p.initial,
            }),
            PrPayload::Association(p) => Self::Association(ParsedPrAssociation {
                repository: log.str(p.repository),
                number: p.number,
                source_sha: log.str(p.source_sha),
                pr_node: log.str(p.pr_node),
            }),
        }
    }

    /// Shared interning order: path, then owner/target or repository/SHA/node.
    pub(crate) fn intern(self, mut intern: impl FnMut(&str) -> StrId) -> PrPayload {
        match self {
            Self::Lifecycle(p) => PrPayload::Lifecycle(PrLifecycle {
                number: p.number,
                owner: intern(p.owner),
                target: p.target.map(&mut intern),
                created: p.created,
                state: p.state,
                initial: p.initial,
            }),
            Self::Association(p) => PrPayload::Association(PrAssociation {
                repository: intern(p.repository),
                number: p.number,
                source_sha: intern(p.source_sha),
                pr_node: intern(p.pr_node),
            }),
        }
    }

    pub(crate) fn is_valid(self, event: ChangeEvent, kind: ChangeKind) -> bool {
        match self {
            Self::Lifecycle(p) => {
                event.pr_kind() == Some(kind)
                    && p.number > 0
                    && is_pr_label(p.owner)
                    && p.target.is_none_or(is_pr_label)
                    && (event == ChangeEvent::PrReviewRequested) == p.target.is_some()
                    && (!p.initial || event == ChangeEvent::PrOpened)
            }
            Self::Association(p) => {
                event == ChangeEvent::Commit
                    && p.number > 0
                    && is_pr_label(p.repository)
                    && is_pr_label(p.pr_node)
                    && p.pr_node.starts_with('/')
                    && matches!(p.source_sha.len(), 40 | 64)
                    && p.source_sha.bytes().all(|b| b.is_ascii_hexdigit())
            }
        }
    }
}

/// Strictly increasing indices local to `ParsedCommit::files`, never table ids.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParsedPrChange<'a> {
    pub file_index: u32,
    pub payload: ParsedPrPayload<'a>,
}

pub(crate) fn valid_pr_changes(commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) -> bool {
    let mut previous = None;
    for record in records {
        if previous.is_some_and(|p| record.file_index <= p) {
            return false;
        }
        previous = Some(record.file_index);
        if commit
            .files
            .get(record.file_index as usize)
            .is_none_or(|file| !record.payload.is_valid(file.event, file.kind))
        {
            return false;
        }
    }
    commit.files.iter().enumerate().all(|(i, f)| {
        f.event.code() <= 3
            || records
                .binary_search_by_key(&(i as u32), |r| r.file_index)
                .is_ok()
    })
}

/// Receiver of parsed commits.
pub trait CommitSink {
    fn commit(&mut self, commit: &ParsedCommit<'_>);

    /// Opt-in git source identity. This is a producer assertion, not provenance
    /// proof. Old sinks retain identical behavior and need no new batch fields.
    fn source_commit(&mut self, commit: &ParsedCommit<'_>, _sha: &str) {
        self.commit(commit);
    }

    /// Old sinks retain their A/M/D fallback. Rich sinks override this seam.
    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, _records: &[ParsedPrChange<'_>]) {
        self.commit(commit);
    }

    /// Frozen log coverage, without a fake event or node.
    fn pr_log(&mut self, _update: PrLogUpdate) {}
}

/// Counters a parser keeps while reading.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub bytes: u64,
    pub lines: u64,
    pub commits: u64,
    pub files: u64,
    /// Lines that did not match the format and were skipped.
    pub skipped_lines: u64,
    /// Attribution data that was ignored: malformed meta pairs of a custom
    /// log, malformed `m1:` lines of git output, and commits whose
    /// attribution set came after the distinct-set cap.
    pub bad_meta: u64,
}
