// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The event stream: an append-only, string-interned store of commits.
//!
//! This replaces Gource's `RCommitLog` + `std::deque<RCommit>` pair. Input
//! adapters (`gource-sources`) append commits as they are parsed, possibly
//! while the simulation is already playing; the simulation reads them through
//! its own cursor. Strings (usernames and file paths) are interned once so a
//! 100k-commit history stays compact and filters can be cached per string.
//!
//! Seeking mirrors Gource's percent-of-file semantics: every commit records the
//! byte offset of its first log line, and [`EventLog::index_at_fraction`] maps
//! a fraction of [`EventLog::total_bytes`] back to a commit index.
//!
//! AI attributions (who was credited on a commit: an agent that assisted a
//! human, or an agent that is the author) are stored as interned *sets*:
//! a commit carries one [`AttrSetId`], and each distinct set of
//! [`Attribution`]s is kept once. Set 0 is the empty set; most logs have
//! only that one.

use std::collections::HashMap;

/// Unix timestamp in seconds (Gource's `time_t`).
pub type Timestamp = i64;

/// Interned string handle; index into [`EventLog`]'s string table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StrId(pub u32);

/// What a commit did to one path (`A`, `M`, `D` in the custom log).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Add,
    Modify,
    Delete,
}

impl ChangeKind {
    /// Parse the custom-log action column the way `RCommitFile` does: `A`
    /// and `M` are themselves, `D` deletes, anything else is treated as a
    /// modification.
    pub fn from_action(action: &str) -> ChangeKind {
        match action {
            "A" => ChangeKind::Add,
            "D" => ChangeKind::Delete,
            _ => ChangeKind::Modify,
        }
    }

    pub fn as_action(self) -> &'static str {
        match self {
            ChangeKind::Add => "A",
            ChangeKind::Modify => "M",
            ChangeKind::Delete => "D",
        }
    }
}

/// What a file change stands for. Commits change files; other project
/// events travel in the same stream as file changes of a pseudo-path
/// (CLAUDE.md: "model 'PR merged' etc. as new event kinds in the same
/// stream as commits"), so they share the clock, filters and recording.
/// A pull request is the path `/pull requests/#123 Title`: opened is an
/// add, merged and closed are deletes. The custom log carries the kind in
/// its meta field as `ev=pr-opened|pr-merged|pr-closed`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChangeEvent {
    /// An ordinary change of a commit.
    #[default]
    Commit,
    PrOpened,
    PrMerged,
    PrClosed,
    ReviewApproved,
    ReviewChangesRequested,
    ReviewCommented,
    PrReopened,
    PrReady,
    PrReviewRequested,
}

impl ChangeEvent {
    /// The batch codec's code. Legacy assignments 0 to 3 never change.
    pub fn code(self) -> u8 {
        match self {
            ChangeEvent::Commit => 0,
            ChangeEvent::PrOpened => 1,
            ChangeEvent::PrMerged => 2,
            ChangeEvent::PrClosed => 3,
            ChangeEvent::ReviewApproved => 4,
            ChangeEvent::ReviewChangesRequested => 5,
            ChangeEvent::ReviewCommented => 6,
            ChangeEvent::PrReopened => 7,
            ChangeEvent::PrReady => 8,
            ChangeEvent::PrReviewRequested => 9,
        }
    }

    pub fn from_code(code: u8) -> Option<ChangeEvent> {
        Some(match code {
            0 => ChangeEvent::Commit,
            1 => ChangeEvent::PrOpened,
            2 => ChangeEvent::PrMerged,
            3 => ChangeEvent::PrClosed,
            4 => ChangeEvent::ReviewApproved,
            5 => ChangeEvent::ReviewChangesRequested,
            6 => ChangeEvent::ReviewCommented,
            7 => ChangeEvent::PrReopened,
            8 => ChangeEvent::PrReady,
            9 => ChangeEvent::PrReviewRequested,
            _ => return None,
        })
    }

    /// The custom log's `ev` value; `None` for a commit change.
    pub fn as_meta(self) -> Option<&'static str> {
        match self {
            ChangeEvent::Commit => None,
            ChangeEvent::PrOpened => Some("pr-opened"),
            ChangeEvent::PrMerged => Some("pr-merged"),
            ChangeEvent::PrClosed => Some("pr-closed"),
            ChangeEvent::ReviewApproved => Some("review-approved"),
            ChangeEvent::ReviewChangesRequested => Some("review-changes-requested"),
            ChangeEvent::ReviewCommented => Some("review-commented"),
            ChangeEvent::PrReopened => Some("pr-reopened"),
            ChangeEvent::PrReady => Some("pr-ready"),
            ChangeEvent::PrReviewRequested => Some("pr-review-requested"),
        }
    }

    /// An `ev` value; unknown values (event kinds of a newer writer) are `None`.
    pub fn from_meta(value: &str) -> Option<ChangeEvent> {
        Some(match value {
            "pr-opened" => ChangeEvent::PrOpened,
            "pr-merged" => ChangeEvent::PrMerged,
            "pr-closed" => ChangeEvent::PrClosed,
            "review-approved" => ChangeEvent::ReviewApproved,
            "review-changes-requested" => ChangeEvent::ReviewChangesRequested,
            "review-commented" => ChangeEvent::ReviewCommented,
            "pr-reopened" => ChangeEvent::PrReopened,
            "pr-ready" => ChangeEvent::PrReady,
            "pr-review-requested" => ChangeEvent::PrReviewRequested,
            _ => return None,
        })
    }

    /// Hover and key text for the event; `None` for a commit change.
    pub fn label(self) -> Option<&'static str> {
        match self {
            ChangeEvent::Commit => None,
            ChangeEvent::PrOpened => Some("Pull request opened"),
            ChangeEvent::PrMerged => Some("Pull request merged"),
            ChangeEvent::PrClosed => Some("Pull request closed"),
            ChangeEvent::ReviewApproved => Some("Review approved"),
            ChangeEvent::ReviewChangesRequested => Some("Review requested changes"),
            ChangeEvent::ReviewCommented => Some("Review commented"),
            ChangeEvent::PrReopened => Some("Pull request reopened"),
            ChangeEvent::PrReady => Some("Pull request ready for review"),
            ChangeEvent::PrReviewRequested => Some("Pull request review requested"),
        }
    }

    /// The approved C++ A/M/D fallback, enforced only for typed rich records.
    pub fn pr_kind(self) -> Option<ChangeKind> {
        match self {
            Self::Commit => None,
            Self::PrOpened | Self::PrReopened => Some(ChangeKind::Add),
            Self::PrMerged | Self::PrClosed => Some(ChangeKind::Delete),
            _ => Some(ChangeKind::Modify),
        }
    }
}

/// One file change inside a commit.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Absolute path, always starting with `/`. A trailing `/` marks a
    /// directory (only meaningful with [`ChangeKind::Delete`]).
    pub path: StrId,
    pub kind: ChangeKind,
    /// Explicit colour from the optional fifth custom-log column.
    pub colour: Option<[u8; 3]>,
    /// What the change stands for ([`ChangeEvent::Commit`] for most).
    pub event: ChangeEvent,
}

/// Initial proposal phase. Review verdicts are separate events, not mergeability.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
}

/// Lifecycle information of one typed PR change. The path remains its node id.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrLifecycle {
    pub number: u64,
    pub owner: StrId,
    pub target: Option<StrId>,
    pub created: Timestamp,
    pub state: PrState,
    pub initial: bool,
}

/// An exact producer annotation, never inferred from a bare PR number.
///
/// The producer must bind repository, commit SHA and target node to its actual
/// code-change receipt. Import validates shape, not provider truth. Consumers
/// must not automatically join or flash files merely because a number matches.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrAssociation {
    pub repository: StrId,
    pub number: u64,
    pub source_sha: StrId,
    pub pr_node: StrId,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrPayload {
    Lifecycle(PrLifecycle),
    Association(PrAssociation),
}

/// Sparse PR metadata keyed by the absolute index in the log's change table.
/// Append APIs accept local indices and offset them exactly once.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrChangeRecord {
    pub change_index: u32,
    pub payload: PrPayload,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrCompleteness {
    Complete,
    Partial,
    Unknown,
}

/// Frozen coverage claims. These describe acquisition, not current PR truth.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PrLogMetadata {
    pub version: u8,
    pub from: Timestamp,
    pub to: Timestamp,
    pub as_of: Timestamp,
    pub lifecycle: PrCompleteness,
    pub reviews: PrCompleteness,
    pub files: PrCompleteness,
}

impl PrLogMetadata {
    pub fn is_valid(self) -> bool {
        self.version == 1 && self.from < self.to
    }
}

/// Unknown is sticky: later markers cannot repair malformed/conflicting input.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PrLogCoverage {
    #[default]
    Absent,
    Known(PrLogMetadata),
    Unknown,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrLogUpdate {
    Metadata(PrLogMetadata),
    Unknown,
}

impl PrLogCoverage {
    pub fn updated(self, update: PrLogUpdate) -> Self {
        match (self, update) {
            (Self::Unknown, _) | (_, PrLogUpdate::Unknown) => Self::Unknown,
            (_, PrLogUpdate::Metadata(m)) if !m.is_valid() => Self::Unknown,
            (Self::Absent, PrLogUpdate::Metadata(m)) => Self::Known(m),
            (Self::Known(old), PrLogUpdate::Metadata(m)) if old == m => Self::Known(old),
            _ => Self::Unknown,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PrDataError {
    InvalidIndex,
    InvalidPayload,
    ConflictingDefinition,
}

/// Bounded, delimiter-independent labels accepted by typed PR payloads.
pub fn is_pr_label(value: &str) -> bool {
    !value.is_empty()
        && value.trim_matches(' ') == value
        && value.len() <= 512
        && !value.chars().any(|c| {
            matches!(c,
        '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{61c}' | '\u{200e}' | '\u{200f}'
        | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

/// How an agent was credited on a commit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AttrKind {
    /// A human's commit credited to the agent (`Co-authored-by`, `Made-with`;
    /// `co` in the custom log).
    Assist,
    /// The agent is the commit's author (`au` in the custom log).
    Agent,
    /// Generic automation account, not AI. Reserved: nothing produces it
    /// before the "bot" milestone (M3), but the batch format accepts it.
    Bot,
}

impl AttrKind {
    /// Code in the batch format: 0 assist, 1 agent, 2 bot.
    pub fn code(self) -> u8 {
        match self {
            AttrKind::Assist => 0,
            AttrKind::Agent => 1,
            AttrKind::Bot => 2,
        }
    }

    pub fn from_code(code: u8) -> Option<AttrKind> {
        match code {
            0 => Some(AttrKind::Assist),
            1 => Some(AttrKind::Agent),
            2 => Some(AttrKind::Bot),
            _ => None,
        }
    }
}

/// One credit of a commit: agent id and optional model text, interned.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Attribution {
    pub kind: AttrKind,
    /// Agent id (`claude`, `codex`, ...; see [`crate::agents`]).
    pub id: StrId,
    /// Model text such as `Claude Opus 5.5`.
    pub model: Option<StrId>,
}

/// Interned attribution set handle; index into [`EventLog`]'s set table.
/// `AttrSetId(0)` ([`AttrSetId::NONE`]) is the empty set.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct AttrSetId(pub u32);

impl AttrSetId {
    pub const NONE: AttrSetId = AttrSetId(0);

    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// Most distinct non-empty attribution sets one log holds. Producers
/// (the parsers in `gource-sources`) stop attributing past it, so every
/// consumer of one stream numbers the same sets.
pub const MAX_ATTR_SETS: usize = 65_536;

/// Most entries in one attribution set.
pub const MAX_SET_ENTRIES: usize = 4;

/// One commit as stored in the log.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CommitRecord {
    pub timestamp: Timestamp,
    pub user: StrId,
    /// Index of the first change in [`EventLog`]'s change table.
    pub first_change: u32,
    pub change_count: u32,
    /// Attribution set of the commit ([`AttrSetId::NONE`] for most).
    pub attrs: AttrSetId,
    /// Byte offset of the commit's first line in the source log.
    pub byte_offset: u64,
}

/// Borrowed view of one commit and its changes.
#[derive(Copy, Clone, Debug)]
pub struct Commit<'a> {
    pub index: usize,
    pub timestamp: Timestamp,
    pub user: StrId,
    pub changes: &'a [FileChange],
    pub attrs: AttrSetId,
    pub byte_offset: u64,
}

/// Append-only commit store with interned strings.
#[derive(Debug, Clone)]
pub struct EventLog {
    strings: Vec<Box<str>>,
    lookup: HashMap<Box<str>, StrId>,
    commits: Vec<CommitRecord>,
    changes: Vec<FileChange>,
    /// Strictly increasing absolute indices; empty and unallocated for plain logs.
    pr_changes: Vec<PrChangeRecord>,
    /// Frozen number/owner/creation definitions per stable PR node path.
    pr_nodes: HashMap<StrId, (u64, StrId, Timestamp)>,
    pr_coverage: PrLogCoverage,
    /// Entries of every set, back to back.
    attr_entries: Vec<Attribution>,
    /// `(start, len)` into `attr_entries` per set id; entry 0 is the empty set.
    attr_sets: Vec<(u32, u32)>,
    attr_lookup: HashMap<Box<[Attribution]>, AttrSetId>,
    /// Commits per set id, entry 0 counting commits without attribution.
    attr_commit_counts: Vec<u32>,
    total_bytes: u64,
    complete: bool,
}

impl Default for EventLog {
    fn default() -> Self {
        EventLog {
            strings: Vec::new(),
            lookup: HashMap::new(),
            commits: Vec::new(),
            changes: Vec::new(),
            pr_changes: Vec::new(),
            pr_nodes: HashMap::new(),
            pr_coverage: PrLogCoverage::Absent,
            attr_entries: Vec::new(),
            attr_sets: vec![(0, 0)],
            attr_lookup: HashMap::new(),
            attr_commit_counts: vec![0],
            total_bytes: 0,
            complete: false,
        }
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything (new repository).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Intern `s`, returning the existing id when already present.
    pub fn intern(&mut self, s: &str) -> StrId {
        if let Some(id) = self.lookup.get(s) {
            return *id;
        }
        self.push_string(s)
    }

    /// Append `s` as a new string without checking for duplicates. Used to
    /// mirror a string table built elsewhere (e.g. in the parsing worker),
    /// where ids must stay identical on both sides.
    pub fn push_string(&mut self, s: &str) -> StrId {
        let id = StrId(self.strings.len() as u32);
        let boxed: Box<str> = s.into();
        self.lookup.entry(boxed.clone()).or_insert(id);
        self.strings.push(boxed);
        id
    }

    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    pub fn str(&self, id: StrId) -> &str {
        &self.strings[id.0 as usize]
    }

    pub fn lookup(&self, s: &str) -> Option<StrId> {
        self.lookup.get(s).copied()
    }

    /// Id of the set holding exactly `entries` (in this order), adding it
    /// when new. Producers pass sets in a canonical order, so equal sets
    /// get equal ids. An empty slice is [`AttrSetId::NONE`], and so is a
    /// new set once [`MAX_ATTR_SETS`] sets exist.
    pub fn intern_attr_set(&mut self, entries: &[Attribution]) -> AttrSetId {
        if entries.is_empty() {
            return AttrSetId::NONE;
        }
        if let Some(&id) = self.attr_lookup.get(entries) {
            return id;
        }
        if self.attr_sets.len() > MAX_ATTR_SETS {
            return AttrSetId::NONE;
        }
        self.push_attr_set(entries)
    }

    /// Append `entries` as a new set without checking for duplicates or the
    /// cap, the set counterpart of [`EventLog::push_string`]: it mirrors a
    /// set table built elsewhere (the parsing worker) with identical ids.
    pub fn push_attr_set(&mut self, entries: &[Attribution]) -> AttrSetId {
        let id = AttrSetId(self.attr_sets.len() as u32);
        let start = self.attr_entries.len() as u32;
        self.attr_entries.extend_from_slice(entries);
        self.attr_sets.push((start, entries.len() as u32));
        self.attr_commit_counts.push(0);
        self.attr_lookup.entry(entries.into()).or_insert(id);
        id
    }

    /// Number of set ids in use, the empty set 0 included (so 1 for a log
    /// without attributions). The next new set gets this id.
    pub fn attr_set_count(&self) -> usize {
        self.attr_sets.len()
    }

    /// Entries of set `id`; empty for [`AttrSetId::NONE`] and unknown ids.
    pub fn attr_set(&self, id: AttrSetId) -> &[Attribution] {
        match self.attr_sets.get(id.0 as usize) {
            Some(&(start, len)) => &self.attr_entries[start as usize..(start + len) as usize],
            None => &[],
        }
    }

    /// Every non-empty set with its id, in id order.
    pub fn attr_sets(&self) -> impl Iterator<Item = (AttrSetId, &[Attribution])> + '_ {
        (1..self.attr_sets.len()).map(|i| {
            let id = AttrSetId(i as u32);
            (id, self.attr_set(id))
        })
    }

    /// Commits stored per set id (index 0: commits without attribution).
    /// The length is [`EventLog::attr_set_count`].
    pub fn attr_commit_counts(&self) -> &[u32] {
        &self.attr_commit_counts
    }

    /// Append a commit. Commits without changes are ignored, as in Gource.
    pub fn push_commit<I>(
        &mut self,
        timestamp: Timestamp,
        user: StrId,
        changes: I,
        byte_offset: u64,
    ) where
        I: IntoIterator<Item = FileChange>,
    {
        self.push_commit_attr(timestamp, user, changes, byte_offset, AttrSetId::NONE);
    }

    /// [`EventLog::push_commit`] with the commit's attribution set, an id
    /// returned by [`EventLog::intern_attr_set`] or
    /// [`EventLog::push_attr_set`]; an unknown id is stored as none.
    pub fn push_commit_attr<I>(
        &mut self,
        timestamp: Timestamp,
        user: StrId,
        changes: I,
        byte_offset: u64,
        attrs: AttrSetId,
    ) where
        I: IntoIterator<Item = FileChange>,
    {
        let first_change = self.changes.len() as u32;
        self.changes.extend(changes);
        let change_count = self.changes.len() as u32 - first_change;
        if change_count == 0 {
            return;
        }
        let attrs = if (attrs.0 as usize) < self.attr_sets.len() {
            attrs
        } else {
            AttrSetId::NONE
        };
        self.commits.push(CommitRecord {
            timestamp,
            user,
            first_change,
            change_count,
            attrs,
            byte_offset,
        });
        let count = &mut self.attr_commit_counts[attrs.0 as usize];
        *count = count.saturating_add(1);
        self.total_bytes = self.total_bytes.max(byte_offset);
    }

    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }

    pub fn change_count(&self) -> usize {
        self.changes.len()
    }

    pub fn pr_change(&self, absolute_index: usize) -> Option<&PrPayload> {
        let index = u32::try_from(absolute_index).ok()?;
        self.pr_changes
            .binary_search_by_key(&index, |r| r.change_index)
            .ok()
            .map(|i| &self.pr_changes[i].payload)
    }

    pub fn pr_changes(&self) -> &[PrChangeRecord] {
        &self.pr_changes
    }

    /// Whether this log carries typed PR lifecycle/association data (not L0).
    pub fn has_pr(&self) -> bool {
        !self.pr_changes.is_empty()
    }

    /// Coverage or exact commit associations alone cannot create a PR flow HUD.
    pub fn has_pr_lifecycle(&self) -> bool {
        !self.pr_nodes.is_empty()
    }

    pub fn pr_coverage(&self) -> PrLogCoverage {
        self.pr_coverage
    }

    /// Frozen definition lookup used for atomic rich import prevalidation.
    pub fn pr_node_definition(&self, path: &str) -> Option<(u64, &str, Timestamp)> {
        let path = self.lookup(path)?;
        self.pr_nodes
            .get(&path)
            .map(|&(number, owner, created)| (number, self.str(owner), created))
    }

    pub fn update_pr_log(&mut self, update: PrLogUpdate) {
        self.pr_coverage = self.pr_coverage.updated(update);
    }

    /// Validate all rich records before appending commits, metadata or offsets.
    pub fn validate_pr_records(
        &self,
        changes: &[FileChange],
        records: &[PrChangeRecord],
    ) -> Result<(), PrDataError> {
        if self
            .changes
            .len()
            .checked_add(changes.len())
            .is_none_or(|n| n > u32::MAX as usize)
        {
            return Err(PrDataError::InvalidIndex);
        }
        let mut previous = None;
        let mut definitions = HashMap::new();
        for record in records {
            if previous.is_some_and(|p| record.change_index <= p) {
                return Err(PrDataError::InvalidIndex);
            }
            previous = Some(record.change_index);
            let file = changes
                .get(record.change_index as usize)
                .ok_or(PrDataError::InvalidIndex)?;
            if file.path.0 as usize >= self.strings.len() {
                return Err(PrDataError::InvalidPayload);
            }
            let label = |id: StrId| {
                self.strings
                    .get(id.0 as usize)
                    .is_some_and(|s| is_pr_label(s))
            };
            match record.payload {
                PrPayload::Lifecycle(p) => {
                    if file.event.pr_kind() != Some(file.kind)
                        || p.number == 0
                        || !label(p.owner)
                        || p.target.is_some_and(|id| !label(id))
                        || (file.event == ChangeEvent::PrReviewRequested) != p.target.is_some()
                        || (p.initial && file.event != ChangeEvent::PrOpened)
                    {
                        return Err(PrDataError::InvalidPayload);
                    }
                    let path = self.lookup(self.str(file.path)).unwrap_or(file.path);
                    let owner = self.lookup(self.str(p.owner)).unwrap_or(p.owner);
                    let identity = (p.number, owner, p.created);
                    if self.pr_nodes.get(&path).is_some_and(|old| *old != identity)
                        || definitions
                            .insert(path, identity)
                            .is_some_and(|old| old != identity)
                    {
                        return Err(PrDataError::ConflictingDefinition);
                    }
                }
                PrPayload::Association(p) => {
                    if file.event != ChangeEvent::Commit
                        || p.number == 0
                        || !label(p.repository)
                        || !label(p.pr_node)
                        || !label(p.source_sha)
                    {
                        return Err(PrDataError::InvalidPayload);
                    }
                    let sha = self.str(p.source_sha);
                    if !self.str(p.pr_node).starts_with('/')
                        || !matches!(sha.len(), 40 | 64)
                        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(PrDataError::InvalidPayload);
                    }
                }
            }
        }
        Ok(())
    }

    /// Add a commit with a sparse local-index PR table. Old append stays unchanged.
    pub fn push_commit_attr_pr(
        &mut self,
        timestamp: Timestamp,
        user: StrId,
        changes: &[FileChange],
        byte_offset: u64,
        attrs: AttrSetId,
        records: &[PrChangeRecord],
    ) -> Result<(), PrDataError> {
        self.validate_pr_records(changes, records)?;
        if user.0 as usize >= self.strings.len() {
            return Err(PrDataError::InvalidPayload);
        }
        let first = self.changes.len() as u32;
        self.push_commit_attr(timestamp, user, changes.iter().copied(), byte_offset, attrs);
        for record in records {
            if let PrPayload::Lifecycle(p) = record.payload {
                let path = changes[record.change_index as usize].path;
                let path = self.lookup(self.str(path)).unwrap_or(path);
                let owner = self.lookup(self.str(p.owner)).unwrap_or(p.owner);
                self.pr_nodes
                    .entry(path)
                    .or_insert((p.number, owner, p.created));
            }
            self.pr_changes.push(PrChangeRecord {
                change_index: first + record.change_index,
                payload: record.payload,
            });
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn commit(&self, index: usize) -> Commit<'_> {
        let rec = &self.commits[index];
        let start = rec.first_change as usize;
        let end = start + rec.change_count as usize;
        Commit {
            index,
            timestamp: rec.timestamp,
            user: rec.user,
            changes: &self.changes[start..end],
            attrs: rec.attrs,
            byte_offset: rec.byte_offset,
        }
    }

    pub fn records(&self) -> &[CommitRecord] {
        &self.commits
    }

    /// Size of the source log in bytes, used as the denominator for seeking.
    /// Grows as commits arrive; set exactly with [`EventLog::set_total_bytes`]
    /// once the whole log has been read.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn set_total_bytes(&mut self, total: u64) {
        self.total_bytes = total;
    }

    /// True once the source has been read to the end. While false the log
    /// behaves like Gource's non-seekable `StreamLog`: running out of commits
    /// means "wait", not "finished".
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn set_complete(&mut self, complete: bool) {
        self.complete = complete;
    }

    /// Commit index at `fraction` (0..=1) of the source log's bytes: the
    /// first commit whose byte offset is at or after the position.
    pub fn index_at_fraction(&self, fraction: f32) -> usize {
        if self.commits.is_empty() {
            return 0;
        }
        let fraction = fraction.clamp(0.0, 1.0) as f64;
        let target = (self.total_bytes as f64 * fraction) as u64;
        let idx = self.commits.partition_point(|c| c.byte_offset < target);
        idx.min(self.commits.len() - 1)
    }

    /// Fraction of the source log consumed once `index` commits have been read.
    pub fn fraction_at_index(&self, index: usize) -> f32 {
        if self.total_bytes == 0 || self.commits.is_empty() {
            return 0.0;
        }
        if index >= self.commits.len() {
            return if self.complete {
                1.0
            } else {
                self.commits[self.commits.len() - 1].byte_offset as f32 / self.total_bytes as f32
            };
        }
        (self.commits[index].byte_offset as f64 / self.total_bytes as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_dedupes_and_mirrors() {
        let mut log = EventLog::new();
        let a = log.intern("alice");
        let b = log.intern("/src/main.rs");
        assert_eq!(log.intern("alice"), a);
        assert_ne!(a, b);
        assert_eq!(log.str(b), "/src/main.rs");

        let mut mirror = EventLog::new();
        assert_eq!(mirror.push_string("alice"), a);
        assert_eq!(mirror.push_string("/src/main.rs"), b);
    }

    #[test]
    fn empty_commits_are_dropped_and_seek_maps_bytes() {
        let mut log = EventLog::new();
        let u = log.intern("u");
        let p = log.intern("/a");
        log.push_commit(10, u, std::iter::empty(), 0);
        log.push_commit(
            10,
            u,
            [FileChange {
                path: p,
                kind: ChangeKind::Add,
                colour: None,
                event: ChangeEvent::Commit,
            }],
            0,
        );
        log.push_commit(
            20,
            u,
            [FileChange {
                path: p,
                kind: ChangeKind::Modify,
                colour: None,
                event: ChangeEvent::Commit,
            }],
            100,
        );
        log.push_commit(
            30,
            u,
            [FileChange {
                path: p,
                kind: ChangeKind::Delete,
                colour: None,
                event: ChangeEvent::Commit,
            }],
            200,
        );
        log.set_total_bytes(300);
        log.set_complete(true);
        assert_eq!(log.commit_count(), 3);
        assert_eq!(log.index_at_fraction(0.0), 0);
        assert_eq!(log.index_at_fraction(0.34), 2);
        assert_eq!(log.index_at_fraction(1.0), 2);
        assert_eq!(log.commit(1).changes[0].kind, ChangeKind::Modify);
        assert_eq!(ChangeKind::from_action("X"), ChangeKind::Modify);
    }

    #[test]
    fn commit_record_stays_32_bytes() {
        assert_eq!(std::mem::size_of::<CommitRecord>(), 32);
    }

    fn change(path: StrId) -> [FileChange; 1] {
        [FileChange {
            path,
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        }]
    }

    #[test]
    fn attribution_sets_are_interned_by_content() {
        let mut log = EventLog::new();
        assert_eq!(log.attr_set_count(), 1);
        assert!(log.attr_set(AttrSetId::NONE).is_empty());
        let u = log.intern("jane");
        let p = log.intern("/a");
        let claude = log.intern("claude");
        let model = log.intern("Claude Opus 5.5");
        let codex = log.intern("codex");
        let a = [Attribution {
            kind: AttrKind::Assist,
            id: claude,
            model: Some(model),
        }];
        let ab = [
            a[0],
            Attribution {
                kind: AttrKind::Assist,
                id: codex,
                model: None,
            },
        ];
        assert_eq!(log.intern_attr_set(&[]), AttrSetId::NONE);
        let s1 = log.intern_attr_set(&a);
        let s2 = log.intern_attr_set(&ab);
        assert_eq!((s1, s2), (AttrSetId(1), AttrSetId(2)));
        assert_eq!(log.intern_attr_set(&a), s1);
        assert_eq!(log.attr_set(s2), &ab);
        assert_eq!(log.attr_set_count(), 3);

        log.push_commit_attr(1, u, change(p), 0, s1);
        log.push_commit_attr(2, u, change(p), 10, s1);
        log.push_commit(3, u, change(p), 20);
        log.push_commit_attr(4, u, std::iter::empty(), 30, s2);
        log.push_commit_attr(5, u, change(p), 40, AttrSetId(99));
        assert_eq!(log.commit(0).attrs, s1);
        assert_eq!(log.commit(2).attrs, AttrSetId::NONE);
        assert_eq!(log.commit(3).attrs, AttrSetId::NONE, "unknown id");
        assert_eq!(log.attr_commit_counts(), &[2, 2, 0]);
        let ids: Vec<_> = log.attr_sets().map(|(id, e)| (id.0, e.len())).collect();
        assert_eq!(ids, [(1, 1), (2, 2)]);

        let mut mirror = EventLog::new();
        assert_eq!(mirror.push_attr_set(&a), s1);
        assert_eq!(mirror.push_attr_set(&ab), s2);
        log.clear();
        assert_eq!(
            (log.attr_set_count(), log.attr_commit_counts()),
            (1, &[0][..])
        );
    }

    #[test]
    fn set_table_is_capped() {
        let mut log = EventLog::new();
        let id = log.intern("x");
        let mut last = AttrSetId::NONE;
        for i in 0..MAX_ATTR_SETS as u32 + 2 {
            let set = [Attribution {
                kind: AttrKind::Assist,
                id,
                model: Some(StrId(i)),
            }];
            last = log.intern_attr_set(&set);
            if i < MAX_ATTR_SETS as u32 {
                assert_eq!(last, AttrSetId(i + 1));
            }
        }
        assert_eq!(last, AttrSetId::NONE);
        assert_eq!(log.attr_set_count(), MAX_ATTR_SETS + 1);
    }
}
