// SPDX-License-Identifier: GPL-3.0-or-later

//! `CustomLogWriter` output format (`Gource::writeCustomLog`).

use std::io::{self, Write};

use gource_core::events::{ChangeEvent, ChangeKind};
use gource_sources::{CommitSink, CustomLogWriter, GitRawParser, ParsedCommit, ParsedFile};

fn file(path: &str, kind: ChangeKind) -> ParsedFile<'_> {
    ParsedFile {
        path,
        kind,
        colour: None,
        event: ChangeEvent::Commit,
    }
}

fn write(commits: &[ParsedCommit<'_>]) -> String {
    let mut w = CustomLogWriter::new(Vec::new());
    for c in commits {
        w.commit(c);
    }
    String::from_utf8(w.finish().unwrap()).unwrap()
}

#[test]
fn one_line_per_file() {
    let files = [
        file("/src/main.rs", ChangeKind::Add),
        file("/README", ChangeKind::Modify),
        file("/old/", ChangeKind::Delete),
    ];
    let out = write(&[ParsedCommit {
        timestamp: 1234567890,
        username: "Jane Doe",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    }]);
    assert_eq!(
        out,
        "1234567890|Jane Doe|A|/src/main.rs\n1234567890|Jane Doe|M|/README\n1234567890|Jane Doe|D|/old/\n"
    );
}

#[test]
fn negative_timestamp_and_colour() {
    let files = [ParsedFile {
        path: "/a.c",
        kind: ChangeKind::Modify,
        colour: Some([0x0a, 0xff, 0x80]),
        event: ChangeEvent::Commit,
    }];
    let out = write(&[ParsedCommit {
        timestamp: -5,
        username: "u",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    }]);
    assert_eq!(out, "-5|u|M|/a.c|0AFF80\n");
}

#[test]
fn separators_and_newlines_are_sanitised() {
    let files = [
        file("/a|b\nc\r.txt", ChangeKind::Add),
        file("|", ChangeKind::Add),
    ];
    let out = write(&[ParsedCommit {
        timestamp: 1,
        username: "x|y\r\nz",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    }]);
    assert_eq!(out, "1|x_yz|A|/a_bc.txt\n1|x_yz|A|_\n");
    for line in out.lines() {
        assert_eq!(line.matches('|').count(), 3, "{line}");
    }
}

#[test]
fn empty_commit_writes_nothing() {
    assert_eq!(
        write(&[ParsedCommit {
            timestamp: 1,
            username: "u",
            files: &[],
            attrs: &[],
            byte_offset: 0
        }]),
        ""
    );
}

/// Fails once more than `limit` bytes have been written.
struct Limited {
    written: Vec<u8>,
    limit: usize,
    fails: usize,
}

impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written.len() + buf.len() > self.limit {
            self.fails += 1;
            return Err(io::Error::other(format!("full #{}", self.fails)));
        }
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn first_io_error_is_kept() {
    let files = [file("/f", ChangeKind::Add)];
    let commit = ParsedCommit {
        timestamp: 1,
        username: "u",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    };
    let mut w = CustomLogWriter::new(Limited {
        written: Vec::new(),
        limit: 12,
        fails: 0,
    });
    w.commit(&commit);
    w.commit(&commit);
    w.commit(&commit);
    assert_eq!(w.error.as_ref().unwrap().to_string(), "full #1");
    assert_eq!(w.out.fails, 1, "nothing is written after the first error");
    assert!(w.out.written.starts_with(b"1|u|A|/f\n1|"));
    assert!(w.out.written.len() <= 12);
    assert_eq!(w.finish().err().unwrap().to_string(), "full #1");
}

#[test]
fn git_raw_to_custom_log() {
    let raw = b"user:A B\n1700000000\n:000000 100644 0 1 A\tsrc/a.rs\n:000000 100644 0 1 A\t\"p|pe\\nx\"\n\n\
user:Merge\n1700000001\nuser:C\n1700000002\n:100644 000000 1 0 D\tsrc/a.rs\n";
    let mut parser = GitRawParser::new();
    let mut w = CustomLogWriter::new(Vec::new());
    parser.push(raw, &mut w);
    parser.finish(&mut w);
    let out = String::from_utf8(w.finish().unwrap()).unwrap();
    assert_eq!(
        out,
        "1700000000|A B|A|/src/a.rs\n1700000000|A B|A|/p_pex\n1700000002|C|D|/src/a.rs\n"
    );
}
