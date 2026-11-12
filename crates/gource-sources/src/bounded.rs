// SPDX-License-Identifier: GPL-3.0-or-later

//! A custom-log writer with a size limit, for histories that are not
//! trusted (a clone service reading an arbitrary repository). A commit whose
//! lines would cross the limit is not written at all: one commit of many
//! files with a long author name expands to `files × name` bytes in a single
//! call, so the check comes before the write.

use std::io::{self, Write};

use crate::{CommitSink, CustomLogWriter, ParsedCommit, ParsedPrChange};

/// Passes writes on to `inner` and counts the bytes it accepted.
pub struct CountingWriter<W: Write> {
    pub inner: W,
    pub bytes: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A [`CustomLogWriter`] that refuses to grow past `max_bytes`; see the
/// module documentation.
pub struct BoundedWriter<W: Write> {
    pub writer: CustomLogWriter<CountingWriter<W>>,
    max_bytes: u64,
    exceeded: bool,
}

impl<W: Write> BoundedWriter<W> {
    /// Upper bound of one log line without the user and path:
    /// `-9223372036854775808|` `|A|` `|RRGGBB` `\n`.
    pub const LINE_OVERHEAD: u64 = 21 + 3 + 7 + 1;
    /// What the meta field adds to the first line of a commit with AI
    /// credits: `|` and at most [`crate::meta::MAX_META_BYTES`] bytes.
    pub const META_OVERHEAD: u64 = 1 + crate::meta::MAX_META_BYTES as u64;

    pub fn new(out: W, max_bytes: u64) -> Self {
        BoundedWriter {
            writer: CustomLogWriter::new(CountingWriter {
                inner: out,
                bytes: 0,
            }),
            max_bytes,
            exceeded: false,
        }
    }

    /// Whether a commit was refused; every later commit is refused too.
    pub fn exceeded(&self) -> bool {
        self.exceeded
    }

    /// Bytes written so far, flushed or not.
    pub fn produced(&self) -> u64 {
        self.writer.out.bytes
    }

    /// Flush and return the output, or the first write error.
    pub fn finish(self) -> io::Result<W> {
        Ok(self.writer.finish()?.inner)
    }
}

impl<W: Write> CommitSink for BoundedWriter<W> {
    fn pr_log(&mut self, update: gource_core::events::PrLogUpdate) {
        self.writer.pr_log(update);
    }

    fn commit_pr(&mut self, commit: &ParsedCommit<'_>, records: &[ParsedPrChange<'_>]) {
        if self.exceeded || self.writer.error.is_some() {
            return;
        }
        let user = (commit.username.len() as u64).saturating_add(Self::LINE_OVERHEAD);
        let bound = commit.files.iter().fold(0u64, |sum, file| {
            sum.saturating_add(user)
                .saturating_add(file.path.len() as u64)
                .saturating_add(Self::META_OVERHEAD)
        });
        if self.produced().saturating_add(bound) > self.max_bytes {
            self.exceeded = true;
            return;
        }
        self.writer.commit_pr(commit, records);
    }

    fn commit(&mut self, commit: &ParsedCommit<'_>) {
        if self.exceeded {
            return;
        }
        let user = commit.username.len() as u64 + Self::LINE_OVERHEAD;
        let meta = if commit.attrs.is_empty() {
            0
        } else {
            Self::META_OVERHEAD
        };
        let bound: u64 = meta
            + commit
                .files
                .iter()
                .map(|f| user + f.path.len() as u64)
                .sum::<u64>();
        if self.produced().saturating_add(bound) > self.max_bytes {
            self.exceeded = true;
            return;
        }
        self.writer.commit(commit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ParsedAttribution, ParsedFile};
    use gource_core::events::{AttrKind, ChangeEvent, ChangeKind};

    fn file(path: &str) -> ParsedFile<'_> {
        ParsedFile {
            path,
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        }
    }

    #[test]
    fn checks_before_writing() {
        let files: Vec<ParsedFile<'_>> = (0..100).map(|_| file("/some/file.txt")).collect();
        let commit = ParsedCommit {
            timestamp: 1_600_000_000,
            username: "A Very Long Author Name",
            files: &files,
            attrs: &[],
            byte_offset: 0,
        };
        let line = "1600000000|A Very Long Author Name|A|/some/file.txt\n".len() as u64;

        let mut w = BoundedWriter::new(Vec::new(), 100 * line * 3);
        w.commit(&commit);
        assert!(!w.exceeded());
        assert_eq!(w.produced(), 100 * line);
        assert_eq!(w.writer.out.inner.len() as u64, 100 * line);

        // Too little room left for the next commit: nothing of it is written.
        let mut w = BoundedWriter::new(Vec::new(), 100 * line + 10);
        w.commit(&commit);
        assert!(w.exceeded());
        assert!(w.writer.out.inner.is_empty());
        w.commit(&commit);
        assert!(w.writer.out.inner.is_empty());
    }

    #[test]
    fn counts_the_meta_field() {
        let files = [file("/a")];
        let attrs = [ParsedAttribution {
            kind: AttrKind::Assist,
            agent: "claude",
            model: Some("Claude Opus 5.5"),
        }];
        let commit = ParsedCommit {
            timestamp: 1,
            username: "u",
            files: &files,
            attrs: &attrs,
            byte_offset: 0,
        };
        type B = BoundedWriter<Vec<u8>>;
        let bound = 1 + B::LINE_OVERHEAD + 2 + B::META_OVERHEAD;
        let mut w = B::new(Vec::new(), bound - 1);
        w.commit(&commit);
        assert!(w.exceeded(), "the bound includes the field");
        let mut w = B::new(Vec::new(), bound);
        w.commit(&commit);
        assert!(!w.exceeded());
        assert_eq!(
            String::from_utf8(w.finish().unwrap()).unwrap(),
            "1|u|A|/a||ai=co:claude:Claude Opus 5.5\n"
        );
    }

    #[test]
    fn without_a_limit_it_is_the_plain_writer() {
        let files = [file("/a"), file("/b")];
        let commit = ParsedCommit {
            timestamp: 7,
            username: "ann",
            files: &files,
            attrs: &[],
            byte_offset: 0,
        };
        let mut plain = CustomLogWriter::new(Vec::new());
        plain.commit(&commit);
        let mut bounded = BoundedWriter::new(Vec::new(), u64::MAX);
        bounded.commit(&commit);
        assert_eq!(bounded.finish().unwrap(), plain.finish().unwrap());
    }
}
