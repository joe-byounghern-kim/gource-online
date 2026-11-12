// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Binary event batches for Web Worker → main thread transfer.
//!
//! The parser runs in a Web Worker and the simulation on the main thread, and
//! each has its own [`EventLog`]. Commits cross between them as compact byte
//! batches ([`BatchEncoder`] on the worker side, [`apply_batch`] on the
//! other). The worker's encoder interns usernames and paths in first-seen
//! order, so the main thread's log ends up with *identical string ids*
//! without any string being sent twice.
//!
//! ```text
//! worker                                                     main thread
//! CustomLogParser ──► BatchEncoder ──► take_batch() ═► apply_batch(&mut EventLog)
//!      (chunks)         intern in first-seen order     (postMessage, transferred)
//! ```
//!
//! # Wire format, version 2
//!
//! All fixed-width integers are little-endian. A `varint` is an unsigned
//! LEB128 number: seven bits per byte, least significant group first, the
//! high bit set while more bytes follow (at most 5 bytes for a `u32`, 10 for
//! a `u64`). A `zigzag` is a varint of `(n << 1) ^ (n >> 63)` for a signed
//! 64-bit `n`, which keeps small deltas of either sign small.
//!
//! ```text
//! batch   = header strings sets commits
//!
//! header  = magic           3 bytes  "GEB"
//!           version         u8       2
//!           first_string    u32      id of the first string in this batch
//!           first_set       u32      id of the first attribution set in it
//!           first_commit    u32      index of the first commit in this batch
//!           string_count    u32      strings in this batch
//!           set_count       u32      attribution sets in this batch
//!           commit_count    u32      commits in this batch
//!           change_count    u32      changes summed over this batch's commits
//!                                                          (32 bytes in all)
//!
//! strings = string_count * ( len: varint,  bytes: [u8; len] )      UTF-8
//!
//! sets    = set_count * (
//!             n: varint           entries, 1 to 4
//!             n * (
//!               kind:  u8         0 assist, 1 agent, 2 bot (reserved)
//!               id:    varint     string id of the agent
//!               model: varint     0 = none, else string id + 1
//!             ) )
//!
//! commits = commit_count * (
//!             timestamp: zigzag   difference to the previous commit's
//!                                 timestamp in this batch (0 before the first)
//!             user:      varint   string id
//!             offset:    zigzag   difference to the previous commit's
//!                                 byte_offset in this batch (0 before the first)
//!             head:      varint   (changes << 1) | has_set; changes >= 1
//!             [set:      varint]  attribution set id (>= 1), when has_set
//!             changes * (
//!               path:  varint     string id
//!               flags: u8         bits 0-1: kind (0 add, 1 modify, 2 delete)
//!                                 bit 2:    a colour follows
//!                                 bits 3-5: event (0 commit change,
//!                                           1 PR opened, 2 PR merged,
//!                                           3 PR closed; ChangeEvent::code)
//!                                 bits 6-7: zero
//!               [r, g, b]: 3 bytes, only when the colour bit is set
//!             ) )
//! ```
//!
//! # Additive wire format, version 3
//!
//! The same 32-byte header is followed by `envelope_len: varint` and that
//! bounded section (at most 512 bytes), then the unchanged strings and sets.
//! An empty envelope changes no coverage. Tag 1 contains version 1, three
//! zigzag timestamps (from, to, as-of) and three completeness bytes (0 complete,
//! 1 partial, 2 unknown). Tag 2 is sticky Unknown. Unknown tags/trailing bytes
//! reject the batch. Conflicting valid claims reduce coverage to Unknown.
//!
//! Each change is `path: varint, flags: u8, event: u8, [colour], payload_len:
//! varint, payload`. Flags use only kind bits 0-1 and colour bit 2. Event codes
//! 0-3 remain identical, 4-9 are the six rich events. Payload sections are at
//! most 512 bytes. Tag 1 is lifecycle: number (u64 varint), owner id, target
//! id+1 or zero, created (zigzag), initial phase (0 open/1 draft), seed (0/1).
//! Tag 2 is association: repository id, number, source SHA id, explicit PR node
//! id. All ids are bounded u32 varints except target+1, which can be 2^32.
//! New event codes require a lifecycle payload. Association is ordinary code
//! metadata only, never a guessed numeric join or duplicate code mutation.
//!
//! Plain streams keep exact v2 bytes. Mixed pending batches are rebuilt from
//! retained canonical heads/changes, not by patching serialized v2 bytes.
//! V3 validates the entire batch, including frozen node definitions, before
//! appending any strings, sets, changes or coverage. Its validation uses bounded
//! temporary decoded records; the unchanged v2 validation remains allocation-free.
//!
//! String ids are global to the stream, not to the batch: the first string of
//! the very first batch is id 0 and each batch continues where the previous
//! one stopped (`first_string` is the number of strings sent before it).
//! Commit indices work the same way (`first_commit`), and so do set ids,
//! except that set 0 is the empty set and is never sent: the first set of
//! the stream is 1 (`first_set` is `log.attr_set_count()`). A decoder
//! therefore needs no state besides the [`EventLog`] it fills, and a lost,
//! repeated or reordered batch is detected: [`apply_batch`] refuses a batch
//! whose `first_string`, `first_set` or `first_commit` differs from the
//! log's string, set or commit count.
//!
//! Interning order, shared with [`crate::EventLogSink`] so that ids agree:
//! per commit the username, then the agent id and model of each entry of
//! its attribution set in set order, then the file paths. Sets are numbered
//! in first-seen order. Version 1 batches (no sets) are rejected.
//!
//! # Guarantees of [`apply_batch`]
//!
//! - **All or nothing.** The whole batch is validated (header, UTF-8, every
//!   id in range, counts, no trailing bytes) before the log is touched, so a
//!   rejected batch leaves the log exactly as it was.
//! - **No panics** on any input, however malformed.
//! - Strings are appended with [`EventLog::push_string`] and sets with
//!   [`EventLog::push_attr_set`], so the ids on both sides agree, and
//!   commits with [`EventLog::push_commit_attr`].
//! - A stream with more than [`MAX_ATTR_SETS`] sets is refused (the
//!   parsers stop attributing before that, so the in-tree encoder never
//!   produces one).
//!
//! # Allocation
//!
//! The encoder keeps its buffers between batches; the only allocation per
//! batch is the returned `Vec` (exactly sized) plus the growth of the string
//! table for strings never seen before. Decoding allocates nothing besides
//! what [`EventLog`] itself stores.

use std::collections::HashMap;

use gource_core::events::{
    AttrKind, AttrSetId, Attribution, ChangeEvent, ChangeKind, EventLog, FileChange, MAX_ATTR_SETS,
    MAX_SET_ENTRIES, PrAssociation, PrChangeRecord, PrCompleteness, PrLifecycle, PrLogCoverage,
    PrLogMetadata, PrLogUpdate, PrPayload, PrState, StrId,
};

use crate::{CommitSink, ParsedCommit, ParsedPrChange, ParsedPrPayload, valid_pr_changes};

/// Export frozen event semantics through the existing batch format. Decode with
/// [`apply_batch`] into a fresh EventLog. IDs are canonically re-interned, not
/// preserved as opaque source IDs. The format does not carry source `total_bytes`
/// or `complete`: restore the original source byte length for percentage seeks
/// and mark complete when the source ends, never use the encoded batch length.
/// Plain logs stay v2. Rich lifecycle, exact associations, F1 and coverage use v3.
/// This is the portable alternative when custom metadata cannot represent proof.
/// No provider calls, invented events or custom-log association inference occur.
pub fn encode_event_log(log: &EventLog) -> Vec<u8> {
    let mut encoder = BatchEncoder::new();
    match log.pr_coverage() {
        PrLogCoverage::Known(m) => encoder.pr_log(PrLogUpdate::Metadata(m)),
        PrLogCoverage::Unknown => encoder.pr_log(PrLogUpdate::Unknown),
        PrLogCoverage::Absent => {}
    }
    for index in 0..log.commit_count() {
        let commit = log.commit(index);
        let files: Vec<_> = commit
            .changes
            .iter()
            .map(|f| crate::ParsedFile {
                path: log.str(f.path),
                kind: f.kind,
                colour: f.colour,
                event: f.event,
            })
            .collect();
        let attrs: Vec<_> = log
            .attr_set(commit.attrs)
            .iter()
            .map(|a| crate::ParsedAttribution {
                kind: a.kind,
                agent: log.str(a.id),
                model: a.model.map(|id| log.str(id)),
            })
            .collect();
        let first = log.records()[index].first_change as usize;
        let records: Vec<_> = (0..files.len())
            .filter_map(|i| {
                log.pr_change(first + i).map(|p| ParsedPrChange {
                    file_index: i as u32,
                    payload: ParsedPrPayload::from_log(log, *p),
                })
            })
            .collect();
        let parsed = ParsedCommit {
            timestamp: commit.timestamp,
            username: log.str(commit.user),
            files: &files,
            attrs: &attrs,
            byte_offset: commit.byte_offset,
        };
        if records.is_empty() {
            encoder.commit(&parsed);
        } else {
            encoder.commit_pr(&parsed, &records);
        }
    }
    encoder.take_batch()
}

const MAGIC: [u8; 3] = *b"GEB";
const VERSION: u8 = 2;
const HEADER_LEN: usize = 32;
const V3: u8 = 3;
const MAX_PR_BYTES: usize = 512;

/// `(kind code, agent string id, model string id + 1 or 0)`.
type SetEntry = (u8, u32, u32);

const KIND_MASK: u8 = 0b011;
const HAS_COLOUR: u8 = 0b100;
const EVENT_SHIFT: u8 = 3;
const EVENT_MASK: u8 = 0b111 << EVENT_SHIFT;

/// Encodes parsed commits into binary batches for transfer between a Web
/// Worker and the main thread.
#[derive(Debug, Default)]
pub struct BatchEncoder {
    /// Every string sent so far, with its id (= first-seen order).
    ids: HashMap<Box<str>, u32>,
    /// Strings first seen in the current batch, already encoded.
    new_strings: Vec<u8>,
    new_string_count: u32,

    /// Every attribution set sent so far, with its set id (1-based,
    /// first-seen order).
    sets: HashMap<Box<[SetEntry]>, u32>,
    /// Sets first seen in the current batch, already encoded.
    new_sets: Vec<u8>,
    new_set_count: u32,

    /// Commits of the current batch, already encoded.
    commits: Vec<u8>,
    commit_count: u32,
    change_count: u32,
    /// Commits in batches taken before the current one.
    commits_before: u32,

    prev_timestamp: i64,
    prev_offset: u64,
    /// Canonical typed pending data, reused to safely promote mixed batches.
    pending_heads: Vec<CommitHead>,
    pending_changes: Vec<(FileChange, Option<PrPayload>)>,
    changes_scratch: Vec<(FileChange, Option<PrPayload>)>,
    rich_stream: bool,
    coverage: PrLogCoverage,
    coverage_dirty: bool,
    pr_nodes: HashMap<Box<str>, (u64, Box<str>, i64)>,
}

impl BatchEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Commits encoded since the last [`BatchEncoder::take_batch`].
    pub fn pending_commits(&self) -> usize {
        self.commit_count as usize
    }

    /// Size in bytes of the batch that [`BatchEncoder::take_batch`] would
    /// return now (0 when nothing is pending). Lets the caller flush once a
    /// batch is big enough.
    pub fn pending_bytes(&self) -> usize {
        if self.commit_count == 0 && !self.coverage_dirty {
            0
        } else {
            let mut bytes =
                HEADER_LEN + self.new_strings.len() + self.new_sets.len() + self.commits.len();
            if self.rich_stream {
                let envelope = coverage_len(self.coverage, self.coverage_dirty);
                bytes += varint_len(envelope as u64) + envelope;
                for (_, payload) in &self.pending_changes {
                    let len = payload_len(*payload);
                    bytes += 1 + varint_len(len as u64) + len;
                }
            }
            bytes
        }
    }

    /// Number of distinct strings interned so far.
    pub fn string_count(&self) -> usize {
        self.ids.len()
    }

    /// Take the encoded batch (strings first seen since the last batch, then
    /// commits). Returns an empty vector when nothing is pending.
    pub fn take_batch(&mut self) -> Vec<u8> {
        if self.commit_count == 0 && !self.coverage_dirty {
            return Vec::new();
        }

        let first_string = self.ids.len() as u32 - self.new_string_count;
        let first_set = self.sets.len() as u32 + 1 - self.new_set_count;
        let mut out = Vec::with_capacity(self.pending_bytes());
        out.extend_from_slice(&MAGIC);
        out.push(if self.rich_stream { V3 } else { VERSION });
        for n in [
            first_string,
            first_set,
            self.commits_before,
            self.new_string_count,
            self.new_set_count,
            self.commit_count,
            self.change_count,
        ] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        if self.rich_stream {
            let envelope = coverage_bytes(self.coverage, self.coverage_dirty);
            put_varint(&mut out, envelope.len() as u64);
            out.extend_from_slice(&envelope);
        }
        out.extend_from_slice(&self.new_strings);
        out.extend_from_slice(&self.new_sets);
        if self.rich_stream {
            let mut previous = Deltas::default();
            let mut first = 0;
            for head in &self.pending_heads {
                write_commit_head(&mut out, head, &mut previous);
                for &(file, payload) in &self.pending_changes[first..first + head.changes as usize]
                {
                    write_v3_change(&mut out, file, payload);
                }
                first += head.changes as usize;
            }
        } else {
            out.extend_from_slice(&self.commits);
        }

        self.commits_before = self.commits_before.wrapping_add(self.commit_count);
        self.new_strings.clear();
        self.new_string_count = 0;
        self.new_sets.clear();
        self.new_set_count = 0;
        self.commits.clear();
        self.commit_count = 0;
        self.change_count = 0;
        self.prev_timestamp = 0;
        self.prev_offset = 0;
        self.pending_heads.clear();
        self.pending_changes.clear();
        self.coverage_dirty = false;
        out
    }

    /// Id of `s`, adding it to the current batch's string section when new.
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.ids.len() as u32;
        put_varint(&mut self.new_strings, s.len() as u64);
        self.new_strings.extend_from_slice(s.as_bytes());
        self.new_string_count += 1;
        self.ids.insert(s.into(), id);
        id
    }

    /// Set id of the commit's attributions (0 for none), interning their
    /// strings first and adding the set to the current batch when new.
    fn intern_set(&mut self, commit: &ParsedCommit<'_>) -> u32 {
        if commit.attrs.is_empty() {
            return 0;
        }
        let mut key: [SetEntry; MAX_SET_ENTRIES] = [(0, 0, 0); MAX_SET_ENTRIES];
        let n = commit.attrs.len().min(MAX_SET_ENTRIES);
        for (slot, a) in key.iter_mut().zip(commit.attrs) {
            let id = self.intern(a.agent);
            let model = a.model.map_or(0, |m| self.intern(m) + 1);
            *slot = (a.kind.code(), id, model);
        }
        let key = &key[..n];
        if let Some(&id) = self.sets.get(key) {
            return id;
        }
        // Mirrors `EventLog::intern_attr_set`, which gives up at the cap.
        if self.sets.len() >= MAX_ATTR_SETS {
            return 0;
        }
        let id = self.sets.len() as u32 + 1;
        put_varint(&mut self.new_sets, n as u64);
        for &(kind, agent, model) in key {
            self.new_sets.push(kind);
            put_varint(&mut self.new_sets, u64::from(agent));
            put_varint(&mut self.new_sets, u64::from(model));
        }
        self.new_set_count += 1;
        self.sets.insert(key.into(), id);
        id
    }
}

impl CommitSink for BatchEncoder {
    fn pr_log(&mut self, update: PrLogUpdate) {
        let coverage = self.coverage.updated(update);
        self.coverage_dirty |= coverage != self.coverage;
        self.coverage = coverage;
        self.rich_stream = true;
    }

    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        if !valid_pr_changes(commit, records) {
            self.pr_log(PrLogUpdate::Unknown);
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
        self.rich_stream |= !records.is_empty() || commit.files.iter().any(|f| f.event.code() > 3);
        self.record_commit(commit, records);
    }

    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        if commit.files.iter().any(|f| f.event.code() > 3) {
            self.commit_pr(commit, &[]);
        } else {
            self.record_commit(commit, &[]);
        }
    }
}

impl BatchEncoder {
    fn record_commit(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        // EventLog ignores commits without changes, and so does the stream.
        if commit.files.is_empty() {
            return;
        }

        let user = self.intern(commit.username);
        let set = self.intern_set(commit);

        let mut converted = std::mem::take(&mut self.changes_scratch);
        converted.clear();
        let mut rich = records.iter().peekable();
        for (i, file) in commit.files.iter().enumerate() {
            let path = StrId(self.intern(file.path));
            let payload = if rich.peek().is_some_and(|r| r.file_index as usize == i) {
                Some(
                    rich.next()
                        .unwrap()
                        .payload
                        .intern(|s| StrId(self.intern(s))),
                )
            } else {
                None
            };
            converted.push((
                FileChange {
                    path,
                    kind: file.kind,
                    colour: file.colour,
                    event: file.event,
                },
                payload,
            ));
        }
        let mut definitions = HashMap::new();
        let mut conflict = false;
        for record in records {
            if let ParsedPrPayload::Lifecycle(p) = record.payload {
                let path = commit.files[record.file_index as usize].path;
                let identity = (p.number, p.owner, p.created);
                conflict |= self
                    .pr_nodes
                    .get(path)
                    .is_some_and(|old| (old.0, old.1.as_ref(), old.2) != identity);
                conflict |= definitions
                    .insert(path, identity)
                    .is_some_and(|old| old != identity);
            }
        }
        if conflict {
            self.pr_log(PrLogUpdate::Unknown);
            converted.retain(|(file, _)| file.event == ChangeEvent::Commit);
            for (_, payload) in &mut converted {
                *payload = None;
            }
        } else {
            for (path, (number, owner, created)) in definitions {
                self.pr_nodes
                    .entry(path.into())
                    .or_insert((number, owner.into(), created));
            }
        }
        if converted.is_empty() {
            self.changes_scratch = converted;
            return;
        }

        let out = &mut self.commits;
        put_zigzag(out, commit.timestamp.wrapping_sub(self.prev_timestamp));
        put_varint(out, u64::from(user));
        put_zigzag(
            out,
            commit.byte_offset.wrapping_sub(self.prev_offset) as i64,
        );
        put_varint(out, ((converted.len() as u64) << 1) | u64::from(set != 0));
        if set != 0 {
            put_varint(out, u64::from(set));
        }
        self.prev_timestamp = commit.timestamp;
        self.prev_offset = commit.byte_offset;

        for (file, _) in &converted {
            let out = &mut self.commits;
            put_varint(out, u64::from(file.path.0));
            let kind = match file.kind {
                ChangeKind::Add => 0,
                ChangeKind::Modify => 1,
                ChangeKind::Delete => 2,
            } | (file.event.code() << EVENT_SHIFT);
            match file.colour {
                Some([r, g, b]) => out.extend_from_slice(&[kind | HAS_COLOUR, r, g, b]),
                None => out.push(kind),
            }
        }

        self.commit_count += 1;
        self.change_count = self.change_count.saturating_add(converted.len() as u32);
        self.pending_heads.push(CommitHead {
            timestamp: commit.timestamp,
            user,
            byte_offset: commit.byte_offset,
            changes: converted.len() as u32,
            set,
        });
        self.pending_changes.extend(converted.iter().copied());
        converted.clear();
        self.changes_scratch = converted;
    }
}

fn varint_len(value: u64) -> usize {
    ((64 - value.leading_zeros()).max(1) as usize).div_ceil(7)
}

fn write_commit_head(out: &mut Vec<u8>, head: &CommitHead, previous: &mut Deltas) {
    put_zigzag(out, head.timestamp.wrapping_sub(previous.timestamp));
    put_varint(out, u64::from(head.user));
    put_zigzag(out, head.byte_offset.wrapping_sub(previous.offset) as i64);
    put_varint(
        out,
        (u64::from(head.changes) << 1) | u64::from(head.set != 0),
    );
    if head.set != 0 {
        put_varint(out, u64::from(head.set));
    }
    previous.timestamp = head.timestamp;
    previous.offset = head.byte_offset;
}

fn payload_len(payload: Option<PrPayload>) -> usize {
    match payload {
        None => 0,
        Some(PrPayload::Lifecycle(p)) => {
            3 + varint_len(p.number)
                + varint_len(u64::from(p.owner.0))
                + varint_len(p.target.map_or(0, |id| u64::from(id.0) + 1))
                + varint_len(((p.created << 1) ^ (p.created >> 63)) as u64)
        }
        Some(PrPayload::Association(p)) => {
            1 + varint_len(u64::from(p.repository.0))
                + varint_len(p.number)
                + varint_len(u64::from(p.source_sha.0))
                + varint_len(u64::from(p.pr_node.0))
        }
    }
}

fn write_payload(out: &mut Vec<u8>, payload: Option<PrPayload>) {
    match payload {
        None => {}
        Some(PrPayload::Lifecycle(p)) => {
            out.push(1);
            put_varint(out, p.number);
            put_varint(out, u64::from(p.owner.0));
            put_varint(out, p.target.map_or(0, |id| u64::from(id.0) + 1));
            put_zigzag(out, p.created);
            out.push(u8::from(p.state == PrState::Draft));
            out.push(u8::from(p.initial));
        }
        Some(PrPayload::Association(p)) => {
            out.push(2);
            put_varint(out, u64::from(p.repository.0));
            put_varint(out, p.number);
            put_varint(out, u64::from(p.source_sha.0));
            put_varint(out, u64::from(p.pr_node.0));
        }
    }
}

fn coverage_len(coverage: PrLogCoverage, dirty: bool) -> usize {
    if !dirty {
        return 0;
    }
    match coverage {
        PrLogCoverage::Absent => 0,
        PrLogCoverage::Unknown => 1,
        PrLogCoverage::Known(m) => {
            5 + [m.from, m.to, m.as_of]
                .into_iter()
                .map(|v| varint_len(((v << 1) ^ (v >> 63)) as u64))
                .sum::<usize>()
        }
    }
}

fn coverage_bytes(coverage: PrLogCoverage, dirty: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if !dirty {
        return out;
    }
    match coverage {
        PrLogCoverage::Absent => {}
        PrLogCoverage::Unknown => out.push(2),
        PrLogCoverage::Known(m) => {
            out.extend_from_slice(&[1, m.version]);
            put_zigzag(&mut out, m.from);
            put_zigzag(&mut out, m.to);
            put_zigzag(&mut out, m.as_of);
            out.extend_from_slice(&[
                flag_code(m.lifecycle),
                flag_code(m.reviews),
                flag_code(m.files),
            ]);
        }
    }
    out
}

fn flag_code(flag: PrCompleteness) -> u8 {
    match flag {
        PrCompleteness::Complete => 0,
        PrCompleteness::Partial => 1,
        PrCompleteness::Unknown => 2,
    }
}

fn write_v3_change(out: &mut Vec<u8>, file: FileChange, payload: Option<PrPayload>) {
    put_varint(out, u64::from(file.path.0));
    let kind = match file.kind {
        ChangeKind::Add => 0,
        ChangeKind::Modify => 1,
        ChangeKind::Delete => 2,
    };
    out.push(kind | if file.colour.is_some() { HAS_COLOUR } else { 0 });
    out.push(file.event.code());
    if let Some(rgb) = file.colour {
        out.extend_from_slice(&rgb);
    }
    put_varint(out, payload_len(payload) as u64);
    write_payload(out, payload);
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_zigzag(out: &mut Vec<u8>, v: i64) {
    put_varint(out, ((v << 1) ^ (v >> 63)) as u64);
}

/// Summary of one applied batch.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchSummary {
    pub strings: u32,
    pub sets: u32,
    pub commits: u32,
    pub changes: u32,
}

/// Malformed or out-of-sequence batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchError(pub String);

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid event batch: {}", self.0)
    }
}

impl std::error::Error for BatchError {}

fn err<T>(message: &str) -> Result<T, BatchError> {
    Err(BatchError(message.into()))
}

/// Bounds-checked cursor over a batch.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn is_at_end(&self) -> bool {
        self.pos == self.buf.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], BatchError> {
        let Some(end) = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()) else {
            return err("truncated");
        };
        let bytes = &self.buf[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, BatchError> {
        match self.buf.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                Ok(b)
            }
            None => err("truncated"),
        }
    }

    fn u32_le(&mut self) -> Result<u32, BatchError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn varint(&mut self) -> Result<u64, BatchError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            // The tenth byte has room for one bit only.
            if shift == 63 && b > 1 {
                return err("varint overflows 64 bits");
            }
            value |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        err("varint longer than 10 bytes")
    }

    fn varint_u32(&mut self) -> Result<u32, BatchError> {
        match u32::try_from(self.varint()?) {
            Ok(v) => Ok(v),
            Err(_) => err("varint overflows 32 bits"),
        }
    }

    fn zigzag(&mut self) -> Result<i64, BatchError> {
        let v = self.varint()?;
        Ok((v >> 1) as i64 ^ -((v & 1) as i64))
    }
}

/// Timestamp and offset deltas, reset for every batch.
#[derive(Default)]
struct Deltas {
    timestamp: i64,
    offset: u64,
}

#[derive(Debug)]
struct CommitHead {
    timestamp: i64,
    user: u32,
    byte_offset: u64,
    changes: u32,
    /// Attribution set id, 0 for none.
    set: u32,
}

fn read_commit_head(r: &mut Reader<'_>, deltas: &mut Deltas) -> Result<CommitHead, BatchError> {
    deltas.timestamp = deltas.timestamp.wrapping_add(r.zigzag()?);
    let user = r.varint_u32()?;
    deltas.offset = deltas.offset.wrapping_add(r.zigzag()? as u64);
    let head = r.varint()?;
    let Ok(changes) = u32::try_from(head >> 1) else {
        return err("varint overflows 32 bits");
    };
    let set = if head & 1 != 0 {
        let set = r.varint_u32()?;
        if set == 0 {
            return err("set id 0 sent explicitly");
        }
        set
    } else {
        0
    };
    Ok(CommitHead {
        timestamp: deltas.timestamp,
        user,
        byte_offset: deltas.offset,
        changes,
        set,
    })
}

/// One set entry: `(kind, agent id, model id + 1 or 0)`.
fn read_set_entry(r: &mut Reader<'_>) -> Result<Attribution, BatchError> {
    let Some(kind) = AttrKind::from_code(r.u8()?) else {
        return err("unknown attribution kind");
    };
    let id = r.varint_u32()?;
    let model = r.varint_u32()?;
    Ok(Attribution {
        kind,
        id: StrId(id),
        model: model.checked_sub(1).map(StrId),
    })
}

fn read_change(r: &mut Reader<'_>) -> Result<FileChange, BatchError> {
    let path = r.varint_u32()?;
    let flags = r.u8()?;
    if flags & !(KIND_MASK | HAS_COLOUR | EVENT_MASK) != 0 {
        return err("reserved flag bits set");
    }
    let code = (flags & EVENT_MASK) >> EVENT_SHIFT;
    let Some(event) = ChangeEvent::from_code(code).filter(|_| code <= 3) else {
        return err("unknown change event");
    };
    let kind = match flags & KIND_MASK {
        0 => ChangeKind::Add,
        1 => ChangeKind::Modify,
        2 => ChangeKind::Delete,
        _ => return err("unknown change kind"),
    };
    let colour = if flags & HAS_COLOUR != 0 {
        let c = r.take(3)?;
        Some([c[0], c[1], c[2]])
    } else {
        None
    };
    Ok(FileChange {
        path: StrId(path),
        kind,
        colour,
        event,
    })
}

/// The changes of one commit, decoded on the fly into `EventLog::push_commit`.
struct ChangeIter<'r, 'a> {
    reader: &'r mut Reader<'a>,
    remaining: u32,
    failed: bool,
}

impl Iterator for ChangeIter<'_, '_> {
    type Item = FileChange;

    fn next(&mut self) -> Option<FileChange> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        match read_change(self.reader) {
            Ok(change) => Some(change),
            Err(_) => {
                self.failed = true;
                self.remaining = 0;
                None
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining as usize, Some(self.remaining as usize))
    }
}

/// Append a batch produced by [`BatchEncoder`] to `log`. Batches must be
/// applied in the order they were taken; see the module documentation for
/// the format and for what is checked. On error the log is unchanged.
pub fn apply_batch(log: &mut EventLog, bytes: &[u8]) -> Result<BatchSummary, BatchError> {
    if bytes.is_empty() {
        return Ok(BatchSummary::default());
    }

    let mut r = Reader::new(bytes);
    if r.take(MAGIC.len())? != MAGIC {
        return err("bad magic");
    }
    let version = r.u8()?;
    if version == V3 {
        return apply_v3(log, bytes);
    }
    if version != VERSION {
        return err("unsupported version");
    }
    let first_string = r.u32_le()?;
    let first_set = r.u32_le()?;
    let first_commit = r.u32_le()?;
    let string_count = r.u32_le()?;
    let set_count = r.u32_le()?;
    let commit_count = r.u32_le()?;
    let change_count = r.u32_le()?;

    if first_string as usize != log.string_count() {
        return Err(BatchError(format!(
            "out of order: batch starts at string {first_string} but the log holds {}",
            log.string_count()
        )));
    }
    if first_set as usize != log.attr_set_count() {
        return Err(BatchError(format!(
            "out of order: batch starts at attribution set {first_set} but the log holds {}",
            log.attr_set_count()
        )));
    }
    if first_commit as usize != log.commit_count() {
        return Err(BatchError(format!(
            "out of order: batch starts at commit {first_commit} but the log holds {}",
            log.commit_count()
        )));
    }
    let Some(id_limit) = first_string.checked_add(string_count) else {
        return err("string ids overflow");
    };
    // Set ids below this exist once the batch is applied.
    let set_limit = u64::from(first_set) + u64::from(set_count);
    if set_limit > MAX_ATTR_SETS as u64 + 1 {
        return err("too many attribution sets");
    }

    // Pass 1: check everything without touching the log.
    let strings_at = r.pos;
    for _ in 0..string_count {
        let len = r.varint()?;
        let Ok(len) = usize::try_from(len) else {
            return err("truncated");
        };
        if std::str::from_utf8(r.take(len)?).is_err() {
            return err("string is not UTF-8");
        }
    }
    let sets_at = r.pos;
    for _ in 0..set_count {
        let n = r.varint()?;
        if !(1..=MAX_SET_ENTRIES as u64).contains(&n) {
            return err("attribution set size out of range");
        }
        for _ in 0..n {
            let a = read_set_entry(&mut r)?;
            if a.id.0 >= id_limit || a.model.is_some_and(|m| m.0 >= id_limit) {
                return err("attribution string id out of range");
            }
        }
    }
    let commits_at = r.pos;
    let mut deltas = Deltas::default();
    let mut changes_seen = 0u64;
    for _ in 0..commit_count {
        let head = read_commit_head(&mut r, &mut deltas)?;
        if head.user >= id_limit {
            return err("user id out of range");
        }
        if u64::from(head.set) >= set_limit {
            return err("attribution set id out of range");
        }
        if head.changes == 0 {
            return err("commit without changes");
        }
        for _ in 0..head.changes {
            if read_change(&mut r)?.path.0 >= id_limit {
                return err("path id out of range");
            }
        }
        changes_seen += u64::from(head.changes);
    }
    if changes_seen != u64::from(change_count) {
        return err("change count does not match");
    }
    if !r.is_at_end() {
        return err("trailing bytes");
    }

    // Pass 2: apply. Everything below was validated above.
    let mut r = Reader::new(&bytes[strings_at..sets_at]);
    for _ in 0..string_count {
        let text = r
            .varint()
            .and_then(|len| r.take(len as usize))
            .ok()
            .and_then(|b| std::str::from_utf8(b).ok());
        match text {
            Some(s) => {
                log.push_string(s);
            }
            None => return err("internal error: string section changed"),
        }
    }

    let mut r = Reader::new(&bytes[sets_at..commits_at]);
    let mut set = [Attribution {
        kind: AttrKind::Assist,
        id: StrId(0),
        model: None,
    }; MAX_SET_ENTRIES];
    for _ in 0..set_count {
        let n = r.varint().unwrap_or(0) as usize;
        if !(1..=MAX_SET_ENTRIES).contains(&n) {
            return err("internal error: set section changed");
        }
        for slot in &mut set[..n] {
            match read_set_entry(&mut r) {
                Ok(a) => *slot = a,
                Err(_) => return err("internal error: set section changed"),
            }
        }
        log.push_attr_set(&set[..n]);
    }

    let mut r = Reader::new(&bytes[commits_at..]);
    let mut deltas = Deltas::default();
    for _ in 0..commit_count {
        let Ok(head) = read_commit_head(&mut r, &mut deltas) else {
            return err("internal error: commit section changed");
        };
        let mut changes = ChangeIter {
            reader: &mut r,
            remaining: head.changes,
            failed: false,
        };
        log.push_commit_attr(
            head.timestamp,
            StrId(head.user),
            &mut changes,
            head.byte_offset,
            AttrSetId(head.set),
        );
        if changes.failed {
            return err("internal error: change section changed");
        }
    }

    Ok(BatchSummary {
        strings: string_count,
        sets: set_count,
        commits: commit_count,
        changes: change_count,
    })
}

fn bounded_section<'a>(r: &mut Reader<'a>) -> Result<Reader<'a>, BatchError> {
    let len = r.varint_u32()? as usize;
    if len > MAX_PR_BYTES {
        return err("PR section exceeds limit");
    }
    Ok(Reader::new(r.take(len)?))
}

fn read_flag(r: &mut Reader<'_>) -> Result<PrCompleteness, BatchError> {
    match r.u8()? {
        0 => Ok(PrCompleteness::Complete),
        1 => Ok(PrCompleteness::Partial),
        2 => Ok(PrCompleteness::Unknown),
        _ => err("unknown PR completeness flag"),
    }
}

fn read_coverage(r: &mut Reader<'_>) -> Result<Option<PrLogUpdate>, BatchError> {
    let mut section = bounded_section(r)?;
    if section.is_at_end() {
        return Ok(None);
    }
    let update = match section.u8()? {
        1 => {
            let meta = PrLogMetadata {
                version: section.u8()?,
                from: section.zigzag()?,
                to: section.zigzag()?,
                as_of: section.zigzag()?,
                lifecycle: read_flag(&mut section)?,
                reviews: read_flag(&mut section)?,
                files: read_flag(&mut section)?,
            };
            if !meta.is_valid() {
                return err("invalid PR log metadata");
            }
            PrLogUpdate::Metadata(meta)
        }
        2 => PrLogUpdate::Unknown,
        _ => return err("unknown PR envelope kind"),
    };
    if !section.is_at_end() {
        return err("trailing PR envelope bytes");
    }
    Ok(Some(update))
}

fn read_v3_change(r: &mut Reader<'_>) -> Result<(FileChange, Option<PrPayload>), BatchError> {
    let path = StrId(r.varint_u32()?);
    let flags = r.u8()?;
    if flags & !(KIND_MASK | HAS_COLOUR) != 0 {
        return err("reserved v3 flag bits set");
    }
    let event = ChangeEvent::from_code(r.u8()?)
        .ok_or_else(|| BatchError("unknown v3 change event".into()))?;
    let kind = match flags & KIND_MASK {
        0 => ChangeKind::Add,
        1 => ChangeKind::Modify,
        2 => ChangeKind::Delete,
        _ => return err("unknown change kind"),
    };
    let colour = if flags & HAS_COLOUR != 0 {
        let b = r.take(3)?;
        Some([b[0], b[1], b[2]])
    } else {
        None
    };
    let mut section = bounded_section(r)?;
    let payload = if section.is_at_end() {
        None
    } else {
        Some(match section.u8()? {
            1 => {
                let number = section.varint()?;
                let owner = StrId(section.varint_u32()?);
                let target = match section.varint()? {
                    0 => None,
                    n => Some(StrId(
                        u32::try_from(n - 1)
                            .map_err(|_| BatchError("PR target id overflows".into()))?,
                    )),
                };
                let created = section.zigzag()?;
                let state = match section.u8()? {
                    0 => PrState::Open,
                    1 => PrState::Draft,
                    _ => return err("unknown PR initial state"),
                };
                let initial = match section.u8()? {
                    0 => false,
                    1 => true,
                    _ => return err("invalid PR initial marker"),
                };
                PrPayload::Lifecycle(PrLifecycle {
                    number,
                    owner,
                    target,
                    created,
                    state,
                    initial,
                })
            }
            2 => PrPayload::Association(PrAssociation {
                repository: StrId(section.varint_u32()?),
                number: section.varint()?,
                source_sha: StrId(section.varint_u32()?),
                pr_node: StrId(section.varint_u32()?),
            }),
            _ => return err("unknown PR payload kind"),
        })
    };
    if !section.is_at_end() {
        return err("trailing PR payload bytes");
    }
    if event.code() > 3 && payload.is_none() {
        return err("new PR event without payload");
    }
    Ok((
        FileChange {
            path,
            kind,
            colour,
            event,
        },
        payload,
    ))
}

/// V3 retains the 32-byte v2 header, then a <=512-byte envelope. Changes use
/// path/flags/explicit-event/[colour]/bounded-payload, without repacking v2.
fn apply_v3(log: &mut EventLog, bytes: &[u8]) -> Result<BatchSummary, BatchError> {
    let mut r = Reader::new(bytes);
    r.take(4)?;
    let first_string = r.u32_le()?;
    let first_set = r.u32_le()?;
    let first_commit = r.u32_le()?;
    let string_count = r.u32_le()?;
    let set_count = r.u32_le()?;
    let commit_count = r.u32_le()?;
    let change_count = r.u32_le()?;
    if first_string as usize != log.string_count()
        || first_set as usize != log.attr_set_count()
        || first_commit as usize != log.commit_count()
    {
        return err("out of order v3 batch");
    }
    let id_limit = first_string
        .checked_add(string_count)
        .ok_or_else(|| BatchError("string ids overflow".into()))?;
    let set_limit = u64::from(first_set) + u64::from(set_count);
    if set_limit > MAX_ATTR_SETS as u64 + 1 {
        return err("too many attribution sets");
    }
    if log
        .change_count()
        .checked_add(change_count as usize)
        .is_none_or(|n| n > u32::MAX as usize)
    {
        return err("change indices overflow");
    }
    let coverage = read_coverage(&mut r)?;
    let mut strings = Vec::new();
    for _ in 0..string_count {
        let len = r.varint_u32()? as usize;
        strings.push(
            std::str::from_utf8(r.take(len)?)
                .map_err(|_| BatchError("string is not UTF-8".into()))?,
        );
    }
    let resolve = |id: StrId| -> Result<&str, BatchError> {
        if id.0 >= id_limit {
            return err("PR string id out of range");
        }
        if id.0 < first_string {
            Ok(log.str(id))
        } else {
            Ok(strings[(id.0 - first_string) as usize])
        }
    };
    let mut sets = Vec::new();
    for _ in 0..set_count {
        let count = r.varint()?;
        if !(1..=MAX_SET_ENTRIES as u64).contains(&count) {
            return err("attribution set size out of range");
        }
        let mut entries = Vec::new();
        for _ in 0..count {
            let entry = read_set_entry(&mut r)?;
            resolve(entry.id)?;
            if let Some(model) = entry.model {
                resolve(model)?;
            }
            entries.push(entry);
        }
        sets.push(entries);
    }
    let mut commits = Vec::new();
    let mut deltas = Deltas::default();
    let mut seen = 0u64;
    let mut definitions = HashMap::new();
    for _ in 0..commit_count {
        let head = read_commit_head(&mut r, &mut deltas)?;
        resolve(StrId(head.user))?;
        if u64::from(head.set) >= set_limit || head.changes == 0 {
            return err("invalid v3 commit head");
        }
        let mut changes = Vec::new();
        let mut records = Vec::new();
        for local_index in 0..head.changes {
            let (file, payload) = read_v3_change(&mut r)?;
            let path = resolve(file.path)?;
            if let Some(payload) = payload {
                let parsed = match payload {
                    PrPayload::Lifecycle(p) => {
                        let owner = resolve(p.owner)?;
                        let target = p.target.map(resolve).transpose()?;
                        let identity = (p.number, owner, p.created);
                        if log
                            .pr_node_definition(path)
                            .is_some_and(|old| old != identity)
                            || definitions
                                .insert(path, identity)
                                .is_some_and(|old| old != identity)
                        {
                            return err("conflicting PR definition");
                        }
                        ParsedPrPayload::Lifecycle(crate::ParsedPrLifecycle {
                            number: p.number,
                            owner,
                            target,
                            created: p.created,
                            state: p.state,
                            initial: p.initial,
                        })
                    }
                    PrPayload::Association(p) => {
                        ParsedPrPayload::Association(crate::ParsedPrAssociation {
                            repository: resolve(p.repository)?,
                            number: p.number,
                            source_sha: resolve(p.source_sha)?,
                            pr_node: resolve(p.pr_node)?,
                        })
                    }
                };
                if !parsed.is_valid(file.event, file.kind) {
                    return err("invalid PR payload semantics");
                }
                records.push(PrChangeRecord {
                    change_index: local_index,
                    payload,
                });
            }
            changes.push(file);
        }
        seen += u64::from(head.changes);
        commits.push((head, changes, records));
    }
    if seen != u64::from(change_count) {
        return err("change count does not match");
    }
    if !r.is_at_end() {
        return err("trailing bytes");
    }
    // All counts, refs, semantic payloads and cross-batch identities validated.
    // No borrowed resolver/definition state remains when mutation begins.
    drop(definitions);
    for text in strings {
        log.push_string(text);
    }
    for set in sets {
        log.push_attr_set(&set);
    }
    for (head, changes, records) in commits {
        log.push_commit_attr_pr(
            head.timestamp,
            StrId(head.user),
            &changes,
            head.byte_offset,
            AttrSetId(head.set),
            &records,
        )
        .map_err(|_| BatchError("internal v3 validation mismatch".into()))?;
    }
    if let Some(update) = coverage {
        log.update_pr_log(update);
    }
    Ok(BatchSummary {
        strings: string_count,
        sets: set_count,
        commits: commit_count,
        changes: change_count,
    })
}
