// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Sink that appends parsed commits to an [`EventLog`].

use gource_core::events::{
    AttrKind, AttrSetId, Attribution, ChangeEvent, ChangeKind, EventLog, FileChange,
    MAX_SET_ENTRIES, PrChangeRecord, PrLogUpdate, StrId,
};

use crate::{CommitSink, ParsedCommit, ParsedFile, ParsedPrChange, valid_pr_changes};

/// Commits with at most this many files are converted on the stack.
const INLINE_CHANGES: usize = 32;

/// Appends commits directly into an [`EventLog`].
///
/// The username is interned first, then the agent id and model of each
/// attribution in set order, then the file names in order, so the string
/// ids (and set ids) are the same as the ones a [`crate::BatchEncoder`]
/// hands out for the same commits.
pub struct EventLogSink<'a> {
    pub log: &'a mut EventLog,
}

impl<'a> EventLogSink<'a> {
    pub fn new(log: &'a mut EventLog) -> Self {
        EventLogSink { log }
    }
}

fn change(log: &mut EventLog, file: &ParsedFile<'_>) -> FileChange {
    FileChange {
        path: log.intern(file.path),
        kind: file.kind,
        colour: file.colour,
        event: file.event,
    }
}

/// Intern the commit's attribution strings and set.
fn intern_attrs(log: &mut EventLog, commit: &ParsedCommit<'_>) -> AttrSetId {
    if commit.attrs.is_empty() {
        return AttrSetId::NONE;
    }
    let mut set = [Attribution {
        kind: AttrKind::Assist,
        id: StrId(0),
        model: None,
    }; MAX_SET_ENTRIES];
    let n = commit.attrs.len().min(MAX_SET_ENTRIES);
    for (slot, a) in set.iter_mut().zip(commit.attrs) {
        *slot = Attribution {
            kind: a.kind,
            id: log.intern(a.agent),
            model: a.model.map(|m| log.intern(m)),
        };
    }
    log.intern_attr_set(&set[..n])
}

impl CommitSink for EventLogSink<'_> {
    fn pr_log(&mut self, update: PrLogUpdate) {
        self.log.update_pr_log(update);
    }

    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        if commit.files.is_empty() {
            return;
        }
        if !valid_pr_changes(commit, records) {
            self.log.update_pr_log(PrLogUpdate::Unknown);
            let files: Vec<_> = commit
                .files
                .iter()
                .copied()
                .filter(|f| f.event == ChangeEvent::Commit)
                .collect();
            self.commit(&ParsedCommit {
                files: &files,
                ..*commit
            });
            return;
        }
        let user = self.log.intern(commit.username);
        let attrs = intern_attrs(self.log, commit);
        let mut changes = Vec::with_capacity(commit.files.len());
        let mut rich = Vec::with_capacity(records.len());
        let mut records = records.iter().peekable();
        for (i, file) in commit.files.iter().enumerate() {
            changes.push(change(self.log, file));
            if records.peek().is_some_and(|p| p.file_index as usize == i) {
                let record = records.next().unwrap();
                rich.push(PrChangeRecord {
                    change_index: i as u32,
                    payload: record.payload.intern(|s| self.log.intern(s)),
                });
            }
        }
        if self
            .log
            .push_commit_attr_pr(
                commit.timestamp,
                user,
                &changes,
                commit.byte_offset,
                attrs,
                &rich,
            )
            .is_err()
        {
            self.log.update_pr_log(PrLogUpdate::Unknown);
            // Retain genuine code records only. Malformed PR records cannot mutate paths.
            let plain = changes
                .into_iter()
                .filter(|f| f.event == ChangeEvent::Commit);
            self.log
                .push_commit_attr(commit.timestamp, user, plain, commit.byte_offset, attrs);
        }
    }

    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        if commit.files.iter().any(|f| f.event.code() > 3) {
            self.commit_pr(commit, &[]);
            return;
        }
        // EventLog ignores commits without changes; do not leave their
        // username behind in the string table either.
        if commit.files.is_empty() {
            return;
        }

        let user = self.log.intern(commit.username);
        let attrs = intern_attrs(self.log, commit);
        let count = commit.files.len();

        // Interning needs `&mut EventLog`, so the changes cannot be produced
        // lazily inside `push_commit`: convert them first.
        if count <= INLINE_CHANGES {
            let mut buf = [FileChange {
                path: StrId(0),
                kind: ChangeKind::Add,
                colour: None,
                event: ChangeEvent::Commit,
            }; INLINE_CHANGES];
            for (slot, file) in buf.iter_mut().zip(commit.files) {
                *slot = change(self.log, file);
            }
            self.log.push_commit_attr(
                commit.timestamp,
                user,
                buf[..count].iter().copied(),
                commit.byte_offset,
                attrs,
            );
        } else {
            let changes: Vec<FileChange> = commit
                .files
                .iter()
                .map(|file| change(self.log, file))
                .collect();
            self.log
                .push_commit_attr(commit.timestamp, user, changes, commit.byte_offset, attrs);
        }
    }
}
