// SPDX-License-Identifier: GPL-3.0-or-later

//! Opt-in source producer seam. Actor allocation and identity policy are external.
//! Output spans count completed accepted LF writes, not parser file/header counts.
//! Both outputs must flush and the process must exit successfully before a caller
//! may trust or atomically publish the exact log/receipt pair.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use gource_sources::{BoundedWriter, CommitSink, ParsedCommit, ParsedPrChange, source};

use crate::args::Args;
use crate::error::CliError;
use crate::streams;

pub(super) const MAX_HEADERS: usize = 150_000;
const MAX_MAP_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ROW_BYTES: u64 = 64 + 1 + 128 + 1;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid source data")
}
fn usage() -> CliError {
    CliError::usage(
        "convert",
        "invalid source conversion options or aliased paths",
    )
}

pub(super) struct Config {
    actors: HashMap<String, String>,
    receipts: ReceiptFile,
}

struct ReceiptFile {
    writer: Option<BufWriter<File>>,
    path: PathBuf,
    accepted: bool,
}
impl Drop for ReceiptFile {
    fn drop(&mut self) {
        // Close before unlinking. A failure never leaves an apparently complete
        // receipt file. SIGKILL can still leave staging, which callers must discard.
        drop(self.writer.take());
        if !self.accepted {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn prepare(
    args: &Args,
    git: bool,
    input: &OsStr,
    output: &OsStr,
) -> Result<Option<Config>, CliError> {
    let (actors, receipts) = match (
        args.value("--source-actors"),
        args.value("--source-receipts"),
    ) {
        (None, None) => return Ok(None),
        (Some(a), Some(r))
            if git
                && args.value("--pulls").is_none()
                && args.value("--pulls-since").is_none()
                && args.value("--pulls-until").is_none()
                && a != "-"
                && r != "-" =>
        {
            (Path::new(a), Path::new(r))
        }
        _ => return Err(usage()),
    };
    let mut paths = vec![actors, receipts];
    if input != "-" {
        paths.push(Path::new(input));
    }
    if output != "-" {
        paths.push(Path::new(output));
    }
    for (i, path) in paths.iter().enumerate() {
        // No devices/FIFOs. A receipt is exclusively created, never overwritten.
        if fs::metadata(path).is_ok_and(|m| !m.is_file()) {
            return Err(usage());
        }
        let key = destination(path).map_err(|_| usage())?;
        for other in &paths[..i] {
            if key == destination(other).map_err(|_| usage())? || hardlink_alias(path, other) {
                return Err(usage());
            }
        }
    }
    let actors = load_actors(actors).map_err(|_| CliError::failure("invalid source actor map"))?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipts)
        .map_err(|_| CliError::failure("cannot create source receipts"))?;
    let owned_receipts = ReceiptFile {
        writer: Some(BufWriter::with_capacity(streams::BUFFER, file)),
        path: receipts.to_owned(),
        accepted: false,
    };
    // Two absent paths may normalize to the same name on the filesystem, even
    // when their lexical keys differed above. With our new receipt inode in
    // place, recheck every other role BEFORE open_output can truncate anything.
    // Install the guard first so a rejection removes only our new sidecar.
    let receipt_metadata = owned_receipts
        .writer
        .as_ref()
        .unwrap()
        .get_ref()
        .metadata()
        .map_err(|_| usage())?;
    let receipt_key = fs::canonicalize(receipts).map_err(|_| usage())?;
    for path in paths.iter().filter(|p| **p != receipts) {
        if destination(path).map_err(|_| usage())? == receipt_key
            || fs::metadata(path).is_ok_and(|m| same_file(&receipt_metadata, &m))
        {
            return Err(usage());
        }
    }
    Ok(Some(Config {
        actors,
        receipts: owned_receipts,
    }))
}

fn destination(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(_) => {
            // A dangling symlink is not a new destination.
            if fs::symlink_metadata(path).is_ok() {
                return Err(invalid());
            }
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path.file_name().ok_or_else(invalid)?;
            Ok(fs::canonicalize(parent)?.join(name))
        }
    }
}
fn hardlink_alias(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => same_file(&a, &b),
        _ => false,
    }
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

fn load_actors(path: &Path) -> io::Result<HashMap<String, String>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_MAP_BYTES {
        return Err(invalid());
    }
    let mut reader = BufReader::with_capacity(streams::BUFFER, file.take(MAX_MAP_BYTES + 1));
    let mut actors = HashMap::new();
    let mut total = 0;
    let mut row = Vec::with_capacity(MAX_ROW_BYTES as usize);
    loop {
        row.clear();
        // Neither an enormous row nor a growing file can cause unbounded reads.
        let n = (&mut reader)
            .take(MAX_ROW_BYTES + 1)
            .read_until(b'\n', &mut row)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_MAP_BYTES
            || n as u64 > MAX_ROW_BYTES
            || row.last() != Some(&b'\n')
            || actors.len() >= MAX_HEADERS
        {
            return Err(invalid());
        }
        row.pop();
        let Some(tab) = row.iter().position(|&b| b == b'\t') else {
            return Err(invalid());
        };
        let (sha, label) = (&row[..tab], &row[tab + 1..]);
        if !source::valid_sha(sha)
            || !(1..=128).contains(&label.len())
            || !label
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(b))
        {
            return Err(invalid());
        }
        let sha = std::str::from_utf8(sha).map_err(|_| invalid())?;
        let label = std::str::from_utf8(label).map_err(|_| invalid())?;
        if !source::safe_text(label) || actors.insert(sha.to_owned(), label.to_owned()).is_some() {
            return Err(invalid());
        }
    }
    Ok(actors)
}

struct FinalLines<W: Write> {
    inner: W,
    lines: u64,
    enabled: bool,
}
impl<W: Write> Write for FinalLines<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let accepted = self.inner.write(bytes)?;
        if self.enabled {
            self.lines += bytes[..accepted].iter().filter(|&&b| b == b'\n').count() as u64;
        }
        Ok(accepted)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(super) struct Sink<W: Write> {
    inner: BoundedWriter<FinalLines<W>>,
    config: Option<Config>,
}
impl<W: Write> Sink<W> {
    pub(super) fn new(writer: W, max_bytes: u64, config: Option<Config>) -> Self {
        let enabled = config.is_some();
        Self {
            inner: BoundedWriter::new(
                FinalLines {
                    inner: writer,
                    lines: 0,
                    enabled,
                },
                max_bytes,
            ),
            config,
        }
    }
    pub(super) fn produced(&self) -> u64 {
        self.inner.produced()
    }
    pub(super) fn exceeded(&self) -> bool {
        self.inner.exceeded()
    }
    pub(super) fn has_error(&self) -> bool {
        self.inner.writer.error.is_some()
    }
    pub(super) fn flush(&mut self) {
        if let Err(error) = self.inner.writer.out.flush() {
            self.inner.writer.error = Some(error);
        }
    }
    pub(super) fn finish(mut self, accept_receipts: bool) -> io::Result<W> {
        // ReceiptFile's drop guard removes receipts if the log flush fails.
        let out = self.inner.finish()?;
        if let Some(config) = &mut self.config {
            config.receipts.writer.as_mut().unwrap().flush()?;
            config.receipts.accepted = accept_receipts;
        }
        Ok(out.inner)
    }
    fn refuse(&mut self) {
        self.inner.writer.error = Some(invalid());
    }
}
impl<W: Write> CommitSink for Sink<W> {
    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        if self.config.is_some() {
            self.refuse();
        } else {
            self.inner.commit(commit);
        }
    }
    fn source_commit(&mut self, commit: &ParsedCommit<'_>, sha: &str) {
        if self.has_error() || self.exceeded() {
            return;
        }
        let Some(config) = &self.config else {
            self.inner.commit(commit);
            return;
        };
        let Some(label) = config.actors.get(sha).cloned() else {
            self.refuse();
            return;
        };
        // All F1 credits were already classified using original/mapped names.
        // Never redact/drop an offending path or credit and pretend equivalence.
        if !source::safe_text(&label)
            || commit.files.iter().any(|f| !source::safe_path(f.path))
            || commit.attrs.iter().any(|a| {
                !source::safe_text(a.agent) || a.model.is_some_and(|m| !source::safe_text(m))
            })
        {
            self.refuse();
            return;
        }
        let start = self.inner.writer.out.inner.lines;
        self.inner.commit(&ParsedCommit {
            username: &label,
            ..*commit
        });
        let end = self.inner.writer.out.inner.lines;
        if self.has_error() || self.exceeded() {
            return;
        }
        if end <= start {
            self.refuse();
            return;
        }
        let writer = self
            .config
            .as_mut()
            .unwrap()
            .receipts
            .writer
            .as_mut()
            .unwrap();
        if let Err(error) = writeln!(
            writer,
            "{{\"schema\":1,\"kind\":\"final-custom-line-span\",\"sha\":\"{sha}\",\"label\":\"{label}\",\"start\":{start},\"end\":{end}}}"
        ) {
            self.inner.writer.error = Some(error);
        }
    }
    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        if self.config.is_some() {
            self.refuse();
        } else {
            self.inner.commit_pr(commit, records);
        }
    }
    fn pr_log(&mut self, update: gource_core::events::PrLogUpdate) {
        self.inner.pr_log(update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gource_core::events::{ChangeEvent, ChangeKind};
    use gource_sources::ParsedFile;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[cfg(unix)]
    fn failed_receipt_config(name: &str, capacity: usize) -> (Config, PathBuf) {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "source-receipt-fault-{}-{name}",
                std::process::id()
            ));
        fs::write(&path, []).unwrap();
        let (writer, reader) = UnixStream::pair().unwrap();
        drop(reader);
        // Actual OS EPIPE through a File handle, without unsafe descriptor code
        // or a production test switch. Only this writer seam is substituted.
        let file = File::from(OwnedFd::from(writer));
        let config = Config {
            actors: HashMap::from([(SHA.into(), "opaque".into())]),
            receipts: ReceiptFile {
                writer: Some(BufWriter::with_capacity(capacity, file)),
                path: path.clone(),
                accepted: false,
            },
        };
        (config, path)
    }

    fn commit<'a>(files: &'a [ParsedFile<'a>]) -> ParsedCommit<'a> {
        ParsedCommit {
            timestamp: 1,
            username: "private@example.test",
            files,
            attrs: &[],
            byte_offset: 0,
        }
    }
    fn file() -> ParsedFile<'static> {
        ParsedFile {
            path: "/safe",
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        }
    }

    #[cfg(unix)]
    #[test]
    fn receipt_write_failure_is_sticky_and_removes_receipts() {
        let (config, path) = failed_receipt_config("write", 1);
        let mut sink = Sink::new(Vec::new(), 1024, Some(config));
        sink.source_commit(&commit(&[file()]), SHA);
        assert!(sink.has_error());
        let bytes = sink.produced();
        sink.source_commit(&commit(&[file()]), SHA);
        assert_eq!(sink.produced(), bytes);
        assert!(sink.finish(true).is_err());
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn receipt_final_flush_failure_invalidates_successful_log_writes() {
        let (config, path) = failed_receipt_config("flush", streams::BUFFER);
        let mut sink = Sink::new(Vec::new(), 1024, Some(config));
        sink.source_commit(&commit(&[file()]), SHA);
        assert!(!sink.has_error(), "receipt still buffered");
        assert_eq!(sink.inner.writer.out.inner.lines, 1);
        assert!(sink.finish(true).is_err());
        assert!(!path.exists());
    }

    struct ShortWriter {
        remaining: usize,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("stopped"));
            }
            let n = bytes.len().min(self.remaining).min(1);
            self.remaining -= n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn completed_lf_count_uses_only_accepted_bytes_during_short_writes() {
        let mut counter = FinalLines {
            inner: ShortWriter { remaining: 3 },
            lines: 0,
            enabled: true,
        };
        assert!(counter.write_all(b"a\nbc\n").is_err());
        assert_eq!(
            counter.lines, 1,
            "partial second line is not a final emitted line"
        );
    }
}
