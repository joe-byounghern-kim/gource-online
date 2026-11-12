// SPDX-License-Identifier: GPL-3.0-or-later

//! The worker side: log bytes in, encoded event batches out.
//!
//! Two input formats are accepted and told apart by their first line:
//!
//! - the custom log (`timestamp|username|A/M/D|/path...`), which the clone
//!   service streams and Gource's `--output-custom-log` writes;
//! - the output of Gource's git log command (`user:` header lines), which a
//!   user produces in their own checkout for a private repository. It goes
//!   through the same [`GitRawParser`] and AI classifier as on the server;
//!   names, emails and trailer text stay inside the parser.

use gource_sources::{BatchEncoder, CommitSink, CustomLogParser, GitRawParser, ParseStats};

/// How many leading bytes are buffered at most before the format is chosen
/// (when no line has ended by then).
const SNIFF_BYTES: usize = 256;

/// The input format, as detected from the first line.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LogFormat {
    /// Not decided yet: no complete first line has arrived.
    Unknown,
    Custom,
    Git,
}

impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            LogFormat::Unknown => "unknown",
            LogFormat::Custom => "custom",
            LogFormat::Git => "git",
        }
    }
}

#[derive(Debug)]
enum Parser {
    /// Leading bytes held back until the first line shows the format.
    Sniffing(Vec<u8>),
    Custom(Box<CustomLogParser>),
    Git(Box<GitRawParser>),
}

/// `LogParser` without the JavaScript bindings.
#[derive(Debug)]
pub struct ChunkParser {
    parser: Parser,
    /// For a custom log: applied to ISO dates without a zone.
    utc_offset: i32,
    encoder: BatchEncoder,
}

impl Default for ChunkParser {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ChunkParser {
    /// `seconds_east_of_utc` is applied to ISO dates without a zone in a
    /// custom log, which Gource reads in local time.
    pub fn new(seconds_east_of_utc: i32) -> Self {
        ChunkParser {
            parser: Parser::Sniffing(Vec::new()),
            utc_offset: seconds_east_of_utc,
            encoder: BatchEncoder::new(),
        }
    }

    /// Feed a chunk; returns the batch of commits it completed (often empty:
    /// the newest commit is held back until a later line or `finish`).
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        if let Parser::Sniffing(head) = &mut self.parser {
            head.extend_from_slice(chunk);
            if !head.contains(&b'\n') && head.len() < SNIFF_BYTES {
                return Vec::new();
            }
            let head = std::mem::take(head);
            self.choose(&head);
            self.feed(&head);
        } else {
            self.feed(chunk);
        }
        self.encoder.take_batch()
    }

    /// End of input: the last line and commit.
    pub fn finish(&mut self) -> Vec<u8> {
        if let Parser::Sniffing(head) = &mut self.parser {
            let head = std::mem::take(head);
            self.choose(&head);
            self.feed(&head);
        }
        let sink: &mut dyn CommitSink = &mut self.encoder;
        match &mut self.parser {
            Parser::Sniffing(_) => {}
            Parser::Custom(p) => p.finish(sink),
            Parser::Git(p) => p.finish(sink),
        }
        self.encoder.take_batch()
    }

    pub fn stats(&self) -> ParseStats {
        match &self.parser {
            Parser::Sniffing(head) => ParseStats {
                bytes: head.len() as u64,
                ..ParseStats::default()
            },
            Parser::Custom(p) => p.stats(),
            Parser::Git(p) => p.stats(),
        }
    }

    pub fn format(&self) -> LogFormat {
        match self.parser {
            Parser::Sniffing(_) => LogFormat::Unknown,
            Parser::Custom(_) => LogFormat::Custom,
            Parser::Git(_) => LogFormat::Git,
        }
    }

    fn choose(&mut self, head: &[u8]) {
        self.parser = if looks_like_git(head) {
            Parser::Git(Box::default())
        } else {
            Parser::Custom(Box::new(
                CustomLogParser::new().with_local_utc_offset(self.utc_offset),
            ))
        };
    }

    fn feed(&mut self, bytes: &[u8]) {
        let sink: &mut dyn CommitSink = &mut self.encoder;
        match &mut self.parser {
            Parser::Sniffing(_) => unreachable!("the format is chosen before feeding"),
            Parser::Custom(p) => p.push(bytes, sink),
            Parser::Git(p) => p.push(bytes, sink),
        }
    }
}

/// Whether the first non-blank line is a `user:` header of Gource's git log
/// command. A UTF-8 byte order mark and leading blank lines are skipped. A
/// custom log line starts with a timestamp, so it never matches.
fn looks_like_git(head: &[u8]) -> bool {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let start = head
        .iter()
        .position(|b| !matches!(b, b'\n' | b'\r' | b' ' | b'\t'))
        .unwrap_or(head.len());
    head[start..].starts_with(b"user:")
}

#[cfg(test)]
mod tests {
    use gource_core::events::{ChangeKind, EventLog};
    use gource_sources::apply_batch;

    use super::*;

    fn parse_in_chunks(log: &[u8], chunk: usize) -> (EventLog, ChunkParser) {
        let mut p = ChunkParser::new(0);
        let mut events = EventLog::new();
        for c in log.chunks(chunk) {
            apply_batch(&mut events, &p.push(c)).unwrap();
        }
        apply_batch(&mut events, &p.finish()).unwrap();
        (events, p)
    }

    #[test]
    fn last_commit_arrives_with_finish() {
        let log = "1|a|A|/x\n2|b|M|/x\n3|c|D|/x";
        let mut p = ChunkParser::new(0);
        let mut events = EventLog::new();
        for chunk in log.as_bytes().chunks(3) {
            apply_batch(&mut events, &p.push(chunk)).unwrap();
        }
        // The unterminated last line is only read by finish().
        assert_eq!(events.commit_count(), 1);
        apply_batch(&mut events, &p.finish()).unwrap();
        assert_eq!(events.commit_count(), 3);
        let s = p.stats();
        assert_eq!(s.commits, 3);
        assert_eq!(s.bytes, log.len() as u64);
        assert_eq!(s.skipped_lines, 0);
        assert_eq!(p.format(), LogFormat::Custom);
    }

    const GIT_LOG: &[u8] = b"user:Jane Doe\n1700000000\n\
m1:Jane Doe\x1fjane@example.com\x1fJane Doe\x1fjane@example.com\x1fCo-authored-by: Claude <noreply@anthropic.com>\x1eAdd main\n\
:000000 100644 0000000 d00491f A\tsrc/main.c\n\
:000000 100644 0000000 d00491e A\tREADME\n\
\n\
user:Bob\n1700000100\n\
:100644 100644 d00491f 1191247 M\tsrc/main.c\n\
:100644 000000 d00491e 0000000 D\tREADME\n";

    #[test]
    fn git_log_output_is_detected_and_parsed_in_any_chunking() {
        let (whole, p) = parse_in_chunks(GIT_LOG, GIT_LOG.len());
        assert_eq!(p.format(), LogFormat::Git);
        assert_eq!(whole.commit_count(), 2);
        assert_eq!(p.stats().files, 4);
        assert_eq!(p.stats().bytes, GIT_LOG.len() as u64);
        for chunk in [1, 2, 3, 7, 64, 255, 256, 257] {
            let (events, p) = parse_in_chunks(GIT_LOG, chunk);
            assert_eq!(p.format(), LogFormat::Git, "chunk {chunk}");
            assert_eq!(events.commit_count(), 2, "chunk {chunk}");
            assert_eq!(p.stats(), parse_in_chunks(GIT_LOG, GIT_LOG.len()).1.stats());
        }
    }

    #[test]
    fn git_log_keeps_names_paths_and_ai_credits_only() {
        let (events, _) = parse_in_chunks(GIT_LOG, 5);
        let first = events.commit(0);
        assert_eq!(events.str(first.user), "Jane Doe");
        let files: Vec<_> = first
            .changes
            .iter()
            .map(|f| (events.str(f.path).to_string(), f.kind))
            .collect();
        assert_eq!(
            files,
            [
                ("/src/main.c".to_string(), ChangeKind::Add),
                ("/README".to_string(), ChangeKind::Add)
            ]
        );
        // The Co-authored-by trailer became a Claude credit.
        assert!(!first.attrs.is_none());
        assert_eq!(events.attr_set(first.attrs).len(), 1);
        assert!(events.commit(1).attrs.is_none());
        // No email or trailer text reached the event log's strings.
        for i in 0..events.string_count() {
            let s = events.str(gource_core::events::StrId(i as u32));
            assert!(!s.contains('@'), "{s:?}");
            assert!(!s.contains("Co-authored-by"), "{s:?}");
        }
    }

    #[test]
    fn detection_skips_a_bom_and_blank_lines() {
        assert!(looks_like_git(b"\xEF\xBB\xBF\n\nuser:x\n1\n"));
        assert!(looks_like_git(b"  user:x"));
        assert!(!looks_like_git(b"1700000000|user:x|A|/a\n"));
        assert!(!looks_like_git(b"2023-01-01T00:00:00Z|x|A|/a\n"));
        assert!(!looks_like_git(b""));
    }

    #[test]
    fn a_short_custom_log_without_a_newline_is_read_at_finish() {
        let (events, p) = parse_in_chunks(b"5|a|A|/x", 2);
        assert_eq!(p.format(), LogFormat::Custom);
        assert_eq!(events.commit_count(), 1);
    }

    #[test]
    fn empty_input_reads_as_an_empty_custom_log() {
        let (events, p) = parse_in_chunks(b"", 1);
        assert_eq!(p.format(), LogFormat::Custom);
        assert_eq!(events.commit_count(), 0);
    }
}
