// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs convert`: git raw log or custom log to custom log.
//!
//! With the limit and pull request options this is the clone service's
//! pipeline (`git log … | gource-rs convert …`): commits are written as
//! they are parsed and flushed after every input chunk, so a reader sees the
//! log grow; a limit stops it with exit status 3.

use std::ffi::{OsStr, OsString};
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use gource_sources::pulls::{self, PrEvent, PullRequestMerger};
use gource_sources::{CommitSink, CustomLogParser, GitRawParser, ParseStats};

#[path = "convert_source.rs"]
mod source;

use crate::args::{self, Args, OptionSpec, Parsed};
use crate::error::CliError;
use crate::streams;

const COMMAND: &str = "convert";

pub const HELP: &str = "\
gource-rs convert: convert a git raw log or a custom log to a custom log

USAGE:
    gource-rs convert --from <git|custom> [OPTIONS] <in> <out|->

ARGS:
    <in>     input file, or '-' for standard input
    <out>    output file (overwritten), or '-' for standard output

OPTIONS:
        --from <git|custom>      format of <in> (required)
        --max-bytes <n>          stop when the log would grow past n bytes
        --max-changes <n>        stop after more than n file changes
        --max-commit-files <n>   stop at a commit of more than n files (git input)
        --pulls <file>           merge pull request events from <file>
        --pulls-since <unix>     drop pull request events before this time
        --pulls-until <unix>     drop pull request events after this time
        --progress-json          report {\"commits\":n,\"bytes\":n} lines on standard error
        --source-actors <file>    opt-in bounded SHA<TAB>opaque-label<LF> map
        --source-receipts <file>  paired new JSONL final-line receipts (git only)
    -h, --help                   print this help

FORMATS:
    git       the output of Gource's git command:
                  git log --reverse --raw --encoding=UTF-8 --no-renames \\
                      --no-show-signature --pretty=format:user:%aN%n%ct
    custom    timestamp|username|A/M/D|/path[|RRGGBB], one line per file change
    pulls     number<TAB>created<TAB>merged<TAB>closed<TAB>author<TAB>title,
              times in unix seconds or '-', one pull request per line

The output is the custom log, one line per file change in input order:
    timestamp|username|A|/path[|RRGGBB]
Names are normalised on the way: paths get a leading '/', '|' inside a name
becomes '_', and line breaks inside a name are dropped. Malformed input lines
are skipped. A one-line summary is printed to standard error.

SOURCE MODE:
    Both source options are required. Use a full lowercase SHA on a source:<SHA>
    line after the timestamp or optional m1 line. Labels replace actors only AFTER
    F1 classification. Hosted producers must always supply PRETTY_AI-equivalent
    m1 evidence, even when identity is captured with AI detection disabled.
    Receipts use schema 1, kind final-custom-line-span, SHA, label and zero-based
    half-open start/end FINAL line spans. They assert producer writes, not SHA
    provenance. Trust only after exit 0, both outputs flush and the exact pair is
    validated. Limits: 150000 map rows/headers, 32 MiB map, 194 bytes/row, 200000
    files/commit, 32 MiB buffered commit, 3M changes and 256 MiB output (flags may
    lower the last three count/byte limits). SHA is lowercase 40/64 hex; labels
    are 1-128 ASCII alphanumerics, _ or -. Map rows require LF, no CR/blank rows.
    Source mode rejects malformed headers/raw records, duplicate/missing SHAs,
    invalid UTF-8 paths, email/token markers, delimiters, backslash, dot segments
    and documented controls/bidi. Known-token model credits reject the whole run,
    not just the credit. Some valid Git paths are refused; the marker denylist
    cannot detect arbitrary secrets. No pulls/custom input or aliased file paths.
    Receipts are exclusively created. Errors remove them where possible, but
    killing the process can leave partial staging which callers MUST discard.

EXIT STATUS:
    0 done, 1 failure, 2 bad command line, 3 a limit was reached
    (standard error then says 'limit exceeded: <what> <limit>').
";

const OPTIONS: &[OptionSpec] = &[
    OptionSpec::value("--from"),
    OptionSpec::value("--max-bytes"),
    OptionSpec::value("--max-changes"),
    OptionSpec::value("--max-commit-files"),
    OptionSpec::value("--pulls"),
    OptionSpec::value("--pulls-since"),
    OptionSpec::value("--pulls-until"),
    OptionSpec::switch("--progress-json"),
    OptionSpec::value("--source-actors"),
    OptionSpec::value("--source-receipts"),
];

/// What one commit may buffer in the git parser once `--max-commit-files`
/// is set (the clone service's `MAX_COMMIT_BYTES`).
const MAX_COMMIT_BYTES: usize = 32 * 1024 * 1024;
/// How often `--progress-json` reports.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Format {
    Git,
    Custom,
}

struct Limits {
    max_bytes: u64,
    max_changes: u64,
    max_commit_files: Option<usize>,
}

type Sink = PullRequestMerger<source::Sink<BufWriter<Box<dyn Write>>>>;

/// The two input parsers behind one interface.
trait Source {
    fn push(&mut self, bytes: &[u8], sink: &mut dyn CommitSink);
    fn finish(&mut self, sink: &mut dyn CommitSink);
    fn stats(&self) -> ParseStats;
    /// Commits read so far, merges included (for progress).
    fn seen(&self) -> u64;
    /// A single commit was over `--max-commit-files` (or its byte bound).
    fn commit_too_large(&self) -> bool;
    fn source_failed(&self) -> bool {
        false
    }
}

impl Source for GitRawParser {
    fn push(&mut self, bytes: &[u8], sink: &mut dyn CommitSink) {
        GitRawParser::push(self, bytes, sink);
    }
    fn finish(&mut self, sink: &mut dyn CommitSink) {
        GitRawParser::finish(self, sink);
    }
    fn stats(&self) -> ParseStats {
        GitRawParser::stats(self)
    }
    fn seen(&self) -> u64 {
        self.headers_seen()
    }
    fn commit_too_large(&self) -> bool {
        self.limit_exceeded()
    }
    fn source_failed(&self) -> bool {
        GitRawParser::source_failed(self)
    }
}

impl Source for CustomLogParser {
    fn push(&mut self, bytes: &[u8], sink: &mut dyn CommitSink) {
        CustomLogParser::push(self, bytes, sink);
    }
    fn finish(&mut self, sink: &mut dyn CommitSink) {
        CustomLogParser::finish(self, sink);
    }
    fn stats(&self) -> ParseStats {
        CustomLogParser::stats(self)
    }
    fn seen(&self) -> u64 {
        CustomLogParser::stats(self).commits
    }
    fn commit_too_large(&self) -> bool {
        false
    }
}

pub fn run(args: Vec<OsString>) -> Result<(), CliError> {
    // Ordinary BrokenPipe intentionally exits 0 in main. Source acquisition must
    // never accept a truncated pair or disclose private data in diagnostics.
    let strict = source_requested(&args);
    run_inner(args).map_err(|error| {
        if strict {
            match error {
                CliError::Limit { .. } => error,
                CliError::Usage { .. } => {
                    CliError::usage(COMMAND, "invalid source conversion options")
                }
                _ => CliError::failure("source conversion failed"),
            }
        } else {
            error
        }
    })
}

/// Mirror the option/value token boundaries, without treating an ordinary
/// positional filename or another option's value as an opt-in privacy mode.
fn source_requested(args: &[OsString]) -> bool {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let Some(text) = arg.to_str() else {
            continue;
        };
        if text == "--" || text == "-h" || text == "--help" {
            break;
        }
        let (name, inline) = text
            .split_once('=')
            .map_or((text, false), |(name, _)| (name, true));
        if matches!(name, "--source-actors" | "--source-receipts") {
            return true;
        }
        if !inline
            && OPTIONS
                .iter()
                .any(|spec| spec.name == name && spec.takes_value)
        {
            rest.next();
        }
    }
    false
}

fn run_inner(args: Vec<OsString>) -> Result<(), CliError> {
    let args = match args::parse(COMMAND, &args, OPTIONS)? {
        Parsed::Help => return streams::print_stdout(HELP),
        Parsed::Args(args) => args,
    };

    let from = match args.value("--from") {
        Some("git") => Format::Git,
        Some("custom") => Format::Custom,
        Some(other) => {
            return Err(CliError::usage(
                COMMAND,
                format!("invalid value '{other}' for --from: expected 'git' or 'custom'"),
            ));
        }
        None => {
            return Err(CliError::usage(
                COMMAND,
                "missing required option '--from' (git or custom)",
            ));
        }
    };
    let mut limits = Limits {
        max_bytes: args
            .uint("--max-bytes", 1..=u32::MAX)?
            .map_or(u64::MAX, u64::from),
        max_changes: args
            .uint("--max-changes", 1..=u32::MAX)?
            .map_or(u64::MAX, u64::from),
        max_commit_files: args
            .uint("--max-commit-files", 1..=u32::MAX)?
            .map(|n| n as usize),
    };
    if limits.max_commit_files.is_some() && from == Format::Custom {
        return Err(CliError::usage(
            COMMAND,
            "--max-commit-files applies to git input only",
        ));
    }
    let progress = args.switch("--progress-json");
    let [input, output] = args.positionals(["<in>", "<out>"])?;
    let source = source::prepare(&args, from == Format::Git, input, output)?;
    let source_mode = source.is_some();
    if source_mode {
        limits.max_bytes = limits.max_bytes.min(256 * 1024 * 1024);
        limits.max_changes = limits.max_changes.min(3_000_000);
        limits.max_commit_files = Some(limits.max_commit_files.unwrap_or(200_000).min(200_000));
    }
    let events = pull_request_events(&args)?;

    if same_file(input, output) {
        return Err(CliError::failure(format!(
            "input and output are the same file ({})",
            streams::describe(input)
        )));
    }

    let mut reader = streams::open_input(input)?;
    let out = streams::open_output(output)?;
    let writer = source::Sink::new(
        BufWriter::with_capacity(streams::BUFFER, out),
        limits.max_bytes,
        source,
    );
    let mut sink: Sink = PullRequestMerger::new(writer, events);

    let (stats, hit) = match from {
        Format::Git => {
            let mut parser = match limits.max_commit_files {
                Some(n) => GitRawParser::with_commit_limits(n, MAX_COMMIT_BYTES),
                None => GitRawParser::new(),
            };
            if source_mode {
                parser.require_source_ids(source::MAX_HEADERS);
            }
            let hit = pump(&mut parser, &mut *reader, &mut sink, &limits, progress)
                .map_err(|e| read_error(input, e))?;
            (Source::stats(&parser), hit)
        }
        Format::Custom => {
            let mut parser = CustomLogParser::new();
            let hit = pump(&mut parser, &mut *reader, &mut sink, &limits, progress)
                .map_err(|e| read_error(input, e))?;
            (Source::stats(&parser), hit)
        }
    };

    let emitted = sink.emitted();
    sink.into_inner()
        .finish(hit.is_none())
        .map_err(|e| CliError::io(format!("cannot write {}", describe_output(output)), e))?;
    if let Some(limit) = hit {
        return Err(limit);
    }
    streams::note(&summary(from, &stats, emitted));
    Ok(())
}

/// Feeds `reader` through `parser` into `sink`, flushing the output after
/// every chunk. Returns the limit that stopped it, if one did; a read error
/// is returned as such. A write error stops the reading and is left in the
/// writer for `finish` to report.
fn pump(
    parser: &mut dyn Source,
    reader: &mut dyn Read,
    sink: &mut Sink,
    limits: &Limits,
    progress: bool,
) -> std::io::Result<Option<CliError>> {
    let mut hit = None;
    let mut last_report = Instant::now();
    streams::feed(reader, |chunk| {
        parser.push(chunk, &mut *sink);
        if parser.source_failed() {
            return false;
        }
        hit = over_limit(&*parser, &*sink, limits);
        if hit.is_some() || sink.inner().has_error() {
            return false;
        }
        sink.inner_mut().flush();
        if sink.inner().has_error() {
            return false;
        }
        if progress && last_report.elapsed() >= PROGRESS_EVERY {
            report(parser.seen(), sink.inner().produced());
            last_report = Instant::now();
        }
        true
    })?;
    if parser.source_failed() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid source input",
        ));
    }
    if hit.is_none() && !sink.inner().has_error() {
        parser.finish(&mut *sink);
        if parser.source_failed() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source input",
            ));
        }
        sink.finish();
        hit = over_limit(&*parser, &*sink, limits);
    }
    if progress {
        report(parser.seen(), sink.inner().produced());
    }
    Ok(hit)
}

/// The first limit reached, in the clone service's order.
fn over_limit(parser: &dyn Source, sink: &Sink, limits: &Limits) -> Option<CliError> {
    if parser.commit_too_large() {
        return Some(CliError::Limit {
            what: "commit_files",
            limit: limits.max_commit_files.unwrap_or(0) as u64,
        });
    }
    if sink.inner().exceeded() {
        return Some(CliError::Limit {
            what: "log_bytes",
            limit: limits.max_bytes,
        });
    }
    if parser.stats().files > limits.max_changes {
        return Some(CliError::Limit {
            what: "changes",
            limit: limits.max_changes,
        });
    }
    None
}

fn report(commits: u64, bytes: u64) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "{{\"commits\":{commits},\"bytes\":{bytes}}}"
    );
}

/// The pull request events of `--pulls`, bounded by `--pulls-since` and
/// `--pulls-until`; none without `--pulls`.
fn pull_request_events(args: &Args) -> Result<Vec<PrEvent>, CliError> {
    let since = unix_seconds(args, "--pulls-since")?;
    let until = unix_seconds(args, "--pulls-until")?;
    let Some(path) = args.value("--pulls") else {
        if since.is_some() || until.is_some() {
            return Err(CliError::usage(
                COMMAND,
                "--pulls-since and --pulls-until need --pulls",
            ));
        }
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(path)
        .map_err(|e| CliError::io(format!("cannot read {path}"), e))?;
    let prs = pulls::parse_lines(&text).map_err(|e| CliError::failure(format!("{path}: {e}")))?;
    Ok(pulls::events(&prs, since, until))
}

fn unix_seconds(args: &Args, name: &'static str) -> Result<Option<i64>, CliError> {
    let Some(text) = args.value(name) else {
        return Ok(None);
    };
    text.parse::<i64>().map(Some).map_err(|_| {
        CliError::usage(
            COMMAND,
            format!("invalid value '{text}' for {name}: expected unix seconds"),
        )
    })
}

fn read_error(input: &OsStr, source: std::io::Error) -> CliError {
    CliError::io(format!("cannot read {}", streams::describe(input)), source)
}

fn describe_output(output: &OsStr) -> String {
    if output == "-" {
        "standard output".to_string()
    } else {
        streams::describe(output)
    }
}

/// Whether `output` names a file that already exists and is `input`, which
/// opening it for writing would empty before it is read.
fn same_file(input: &OsStr, output: &OsStr) -> bool {
    if input == "-" || output == "-" {
        return false;
    }
    match (
        std::fs::canonicalize(Path::new(input)),
        std::fs::canonicalize(Path::new(output)),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn summary(from: Format, stats: &ParseStats, pull_request_events: usize) -> String {
    let format = match from {
        Format::Git => "git",
        Format::Custom => "custom",
    };
    let mut text = format!(
        "converted {} commits, {} file changes from {format} input",
        stats.commits, stats.files
    );
    if pull_request_events > 0 {
        text.push_str(&format!(", {pull_request_events} pull request events"));
    }
    if stats.skipped_lines > 0 {
        text.push_str(&format!(" ({} lines skipped)", stats.skipped_lines));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_mentions_skipped_lines_only_when_there_are_some() {
        let mut stats = ParseStats {
            commits: 2,
            files: 5,
            ..ParseStats::default()
        };
        assert_eq!(
            summary(Format::Git, &stats, 0),
            "converted 2 commits, 5 file changes from git input"
        );
        stats.skipped_lines = 3;
        assert_eq!(
            summary(Format::Custom, &stats, 0),
            "converted 2 commits, 5 file changes from custom input (3 lines skipped)"
        );
        assert_eq!(
            summary(
                Format::Git,
                &ParseStats {
                    commits: 1,
                    files: 1,
                    ..ParseStats::default()
                },
                2
            ),
            "converted 1 commits, 1 file changes from git input, 2 pull request events"
        );
    }

    #[test]
    fn stdio_is_never_the_same_file() {
        assert!(!same_file(OsStr::new("-"), OsStr::new("-")));
        assert!(!same_file(OsStr::new("-"), OsStr::new("/dev/null")));
        assert!(!same_file(
            OsStr::new("/definitely/not/here"),
            OsStr::new("/definitely/not/here")
        ));
    }
}
