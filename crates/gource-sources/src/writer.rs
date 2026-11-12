// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Custom log writer (`Gource::writeCustomLog`).
//!
//! Typed rich writes preflight every encoded field before writing the commit.
//! Required identities, lifecycle fields and F1 credits are never trimmed.
//! Optional coverage-carrier omission is counted in `coverage_unknown`.
//! Sticky Unknown coverage cannot be represented faithfully by approved custom
//! grammar: a rich request fails and callers must not deliver partial `out`.
//! `write_event_log_reduced` is an explicit, warned, lossy retry on a fresh
//! writer, not a lossless rich export. V3 preserves Unknown coverage and exact
//! associations. Approved custom metadata cannot represent exact association proof,
//! so rich custom export rejects it rather than silently stripping it.

use std::fmt::Write as _;
use std::io::{self, Write};

use gource_core::events::{ChangeEvent, EventLog, PrLogCoverage, PrLogUpdate};

use crate::meta::{MAX_META_BYTES, MAX_PAIRS, escape_into, write_ai_field, write_pr_log};
use crate::{
    CommitSink, ParsedAttribution, ParsedCommit, ParsedFile, ParsedPrChange, ParsedPrPayload,
    valid_pr_changes,
};

/// Counters a [`CustomLogWriter`] keeps.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteStats {
    /// AI credits left out of a meta field: trailing credits dropped so the
    /// field fits its 512-byte limit, and credits a reader would reject.
    pub meta_trimmed: u64,
    /// Coverage omissions, including a current Known claim not yet emitted.
    /// That pending warning clears if a later eligible carrier is emitted.
    pub coverage_unknown: u64,
    /// Optional associations omitted because custom lacks a trusted source scope.
    pub association_omitted: u64,
}

/// Writes commits in the custom log format, one line per file, the way
/// `gource --output-custom-log` does:
///
/// ```text
/// timestamp|username|A|/path[|RRGGBB]
/// timestamp|username|A|/path|[RRGGBB]|ai=co:claude:Claude Opus 5.5
/// ```
///
/// Unlike `writeCustomLog`, `|` inside usernames and paths is replaced with
/// `_` and carriage returns and newlines are dropped, so every line stays
/// parseable by the custom log parser. A colour column is written only for
/// files that carry an explicit colour, except on lines with a meta field
/// ([`crate::meta`], which C++ Gource ignores): the first line of a commit
/// with AI credits (`ai=...`) and every pull request event (`ev=...`, for
/// example `ev=pr-merged`) get the colour slot (empty without a colour)
/// and then the field. The other lines carry no field.
///
/// The first IO error is kept in [`CustomLogWriter::error`]; nothing is
/// written after it.
pub struct CustomLogWriter<W: Write> {
    pub out: W,
    pub error: Option<std::io::Error>,
    stats: WriteStats,
    meta: String,
    /// The meta field of one line: `ev=...` and, on a commit's first line,
    /// the AI credits.
    line_meta: String,
    coverage: PrLogCoverage,
    carrier_attempted: bool,
    carrier_emitted: bool,
}

impl<W: Write> CustomLogWriter<W> {
    pub fn new(out: W) -> Self {
        CustomLogWriter {
            out,
            error: None,
            stats: WriteStats::default(),
            meta: String::new(),
            line_meta: String::new(),
            coverage: PrLogCoverage::Absent,
            carrier_attempted: false,
            carrier_emitted: false,
        }
    }

    pub fn stats(&self) -> WriteStats {
        let mut stats = self.stats;
        // A Known claim without an emitted carrier is observable even through
        // the streaming seam or with no records. This pending warning clears
        // only if a later eligible opening actually carries that same claim.
        if matches!(self.coverage, PrLogCoverage::Known(_)) && !self.carrier_emitted {
            stats.coverage_unknown += 1;
        }
        stats
    }

    /// Flush the output and return it, or the first error seen.
    pub fn finish(mut self) -> io::Result<W> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(self.out)
    }

    /// Export the frozen log, including its sparse typed data. No provider calls.
    pub fn write_event_log(&mut self, log: &EventLog) {
        if log
            .pr_changes()
            .iter()
            .any(|r| matches!(r.payload, gource_core::events::PrPayload::Association(_)))
        {
            self.error = Some(rich_error(
                "exact associations require lossless v3 export or explicit reduced export",
            ));
            return;
        }
        match log.pr_coverage() {
            PrLogCoverage::Known(m) => self.pr_log(PrLogUpdate::Metadata(m)),
            PrLogCoverage::Unknown => self.pr_log(PrLogUpdate::Unknown),
            PrLogCoverage::Absent => {}
        }
        if self.error.is_some() {
            return;
        }
        for index in 0..log.commit_count() {
            let commit = log.commit(index);
            let files: Vec<_> = commit
                .changes
                .iter()
                .map(|f| ParsedFile {
                    path: log.str(f.path),
                    kind: f.kind,
                    colour: f.colour,
                    event: f.event,
                })
                .collect();
            let attrs: Vec<_> = log
                .attr_set(commit.attrs)
                .iter()
                .map(|a| ParsedAttribution {
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
                self.commit(&parsed);
            } else {
                self.commit_pr(&parsed, &records);
            }
        }
    }

    /// Explicit lossy retry: keep genuine code/AI and free L0, never reinterpret
    /// typed lifecycle rows as code. This is not a successful rich round trip.
    pub fn write_event_log_reduced(&mut self, log: &EventLog) {
        self.stats.coverage_unknown += 1;
        for index in 0..log.commit_count() {
            let commit = log.commit(index);
            let first = log.records()[index].first_change as usize;
            let files: Vec<_> = commit
                .changes
                .iter()
                .enumerate()
                .filter_map(|(i, file)| {
                    if matches!(
                        log.pr_change(first + i),
                        Some(gource_core::events::PrPayload::Lifecycle(_))
                    ) || file.event.code() > 3
                    {
                        return None;
                    }
                    if matches!(
                        log.pr_change(first + i),
                        Some(gource_core::events::PrPayload::Association(_))
                    ) {
                        self.stats.association_omitted += 1;
                    }
                    Some(ParsedFile {
                        path: log.str(file.path),
                        kind: file.kind,
                        colour: file.colour,
                        event: file.event,
                    })
                })
                .collect();
            let attrs: Vec<_> = log
                .attr_set(commit.attrs)
                .iter()
                .map(|a| ParsedAttribution {
                    kind: a.kind,
                    agent: log.str(a.id),
                    model: a.model.map(|id| log.str(id)),
                })
                .collect();
            self.commit(&ParsedCommit {
                timestamp: commit.timestamp,
                username: log.str(commit.user),
                files: &files,
                attrs: &attrs,
                byte_offset: commit.byte_offset,
            });
        }
    }

    fn write_rich_commit(
        &mut self,
        commit: &ParsedCommit<'_>,
        records: &[ParsedPrChange<'_>],
    ) -> io::Result<()> {
        if !valid_pr_changes(commit, records) {
            return Err(rich_error("invalid rich payload"));
        }
        if write_ai_field(&mut self.meta, commit.attrs) != 0 {
            return Err(rich_error("rich attribution does not fit unchanged"));
        }
        let mut fields = Vec::with_capacity(commit.files.len());
        let mut records = records.iter().peekable();
        let mut emits_carrier = false;
        for (i, file) in commit.files.iter().enumerate() {
            let mut field = String::new();
            if let Some(event) = file.event.as_meta() {
                let _ = write!(field, "ev={event}");
            }
            let payload = if records.peek().is_some_and(|r| r.file_index as usize == i) {
                Some(records.next().unwrap().payload)
            } else {
                None
            };
            if let Some(ParsedPrPayload::Lifecycle(p)) = payload {
                let _ = write!(field, ";pr={};pr-author=", p.number);
                escape_into(&mut field, p.owner, false);
                if let Some(target) = p.target {
                    field.push_str(";pr-target=");
                    escape_into(&mut field, target, false);
                }
                let phase = if p.state == gource_core::events::PrState::Draft {
                    "draft"
                } else {
                    "open"
                };
                let _ = write!(
                    field,
                    ";pr-created={};pr-state={phase};pr-initial={}",
                    p.created,
                    u8::from(p.initial)
                );
            } else if matches!(payload, Some(ParsedPrPayload::Association(_))) {
                // Bare `pr` cannot preserve repository/SHA/node proof. Never silently downgrade.
                return Err(rich_error(
                    "exact associations require lossless v3 export or explicit reduced export",
                ));
            }
            if i == 0 && !self.meta.is_empty() {
                if !field.is_empty() {
                    field.push(';');
                }
                field.push_str(&self.meta);
            }
            if !field_fits(&field) {
                return Err(rich_error("mandatory rich metadata exceeds budget"));
            }
            if matches!(payload, Some(ParsedPrPayload::Lifecycle(_)))
                && file.event == ChangeEvent::PrOpened
                && !self.carrier_attempted
                && matches!(self.coverage, PrLogCoverage::Known(_))
            {
                self.carrier_attempted = true;
                if let PrLogCoverage::Known(metadata) = self.coverage {
                    let mut value = String::new();
                    write_pr_log(&mut value, metadata);
                    let before = field.len();
                    field.push_str(";pr-log=");
                    field.push_str(&value);
                    if !field_fits(&field) {
                        field.truncate(before);
                    } else {
                        emits_carrier = true;
                    }
                }
            }
            fields.push(field);
        }
        // Nothing is written until every mandatory encoded field passed.
        for (file, field) in commit.files.iter().zip(fields) {
            write!(self.out, "{}|", commit.timestamp)?;
            write_field(&mut self.out, commit.username)?;
            write!(self.out, "|{}|", file.kind.as_action())?;
            write_field(&mut self.out, file.path)?;
            match file.colour {
                Some([r, g, b]) => write!(self.out, "|{r:02X}{g:02X}{b:02X}")?,
                None if !field.is_empty() => self.out.write_all(b"|")?,
                None => {}
            }
            if !field.is_empty() {
                write!(self.out, "|{field}")?;
            }
            self.out.write_all(b"\n")?;
        }
        self.carrier_emitted |= emits_carrier;
        Ok(())
    }

    fn write_commit(&mut self, commit: &ParsedCommit<'_>) -> io::Result<()> {
        self.meta.clear();
        if !commit.attrs.is_empty() {
            self.stats.meta_trimmed += write_ai_field(&mut self.meta, commit.attrs) as u64;
        }
        for (i, file) in commit.files.iter().enumerate() {
            write!(self.out, "{}|", commit.timestamp)?;
            write_field(&mut self.out, commit.username)?;
            self.out.write_all(b"|")?;
            self.out.write_all(file.kind.as_action().as_bytes())?;
            self.out.write_all(b"|")?;
            write_field(&mut self.out, file.path)?;
            self.line_meta.clear();
            if let Some(ev) = file.event.as_meta() {
                self.line_meta.push_str("ev=");
                self.line_meta.push_str(ev);
            }
            if i == 0 && !self.meta.is_empty() {
                if self.line_meta.len() + 1 + self.meta.len() > MAX_META_BYTES {
                    // A reader ignores a field over the limit as a whole.
                    self.stats.meta_trimmed += commit.attrs.len() as u64;
                } else {
                    if !self.line_meta.is_empty() {
                        self.line_meta.push(';');
                    }
                    self.line_meta.push_str(&self.meta);
                }
            }
            let meta = !self.line_meta.is_empty();
            match file.colour {
                Some([r, g, b]) => write!(self.out, "|{r:02X}{g:02X}{b:02X}")?,
                None if meta => self.out.write_all(b"|")?,
                None => {}
            }
            if meta {
                self.out.write_all(b"|")?;
                self.out.write_all(self.line_meta.as_bytes())?;
            }
            self.out.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn field_fits(field: &str) -> bool {
    field.len() <= MAX_META_BYTES && (field.is_empty() || field.split(';').count() <= MAX_PAIRS)
}

fn rich_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Write `s` with `|` → `_` and `\r`/`\n` removed, without allocating.
fn write_field<W: Write>(out: &mut W, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if matches!(b, b'|' | b'\r' | b'\n') {
            out.write_all(&bytes[start..i])?;
            if b == b'|' {
                out.write_all(b"_")?;
            }
            start = i + 1;
        }
    }
    out.write_all(&bytes[start..])
}

impl<W: Write> CommitSink for CustomLogWriter<W> {
    fn pr_log(&mut self, update: PrLogUpdate) {
        self.coverage = self.coverage.updated(update);
        if self.coverage == PrLogCoverage::Unknown && self.error.is_none() {
            // Custom has no lossless sticky-Unknown carrier. Never invent a range
            // or deliver a partial stream as successful rich output.
            self.stats.coverage_unknown += 1;
            self.error = Some(rich_error(
                "unsupported frozen PR coverage for rich custom export",
            ));
        }
    }

    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.write_rich_commit(commit, records) {
            self.error = Some(error);
        }
    }

    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        if self.error.is_some() {
            return;
        }
        if !valid_pr_changes(commit, &[]) {
            self.error = Some(rich_error("missing mandatory rich payload"));
            return;
        }
        if let Err(e) = self.write_commit(commit) {
            self.error = Some(e);
        }
    }
}
