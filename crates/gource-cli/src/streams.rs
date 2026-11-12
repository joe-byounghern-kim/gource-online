// SPDX-License-Identifier: GPL-3.0-or-later

//! Files and standard streams: `-` means standard input or output.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::error::CliError;

/// Size of the read buffer and of output buffers.
pub const BUFFER: usize = 64 * 1024;

/// A path for messages: the quoted path, or "standard input" for `-`.
pub fn describe(path: &OsStr) -> String {
    if path == "-" {
        "standard input".to_string()
    } else {
        format!("'{}'", Path::new(path).display())
    }
}

/// Open `path` for reading; `-` is standard input.
pub fn open_input(path: &OsStr) -> Result<Box<dyn Read>, CliError> {
    open_input_with(path, |path| File::open(path))
}

/// Keep the OS path intact at the file-opening boundary. An injectable opener
/// also lets tests exercise raw-byte paths on filesystems that reject them.
fn open_input_with(
    path: &OsStr,
    open: impl FnOnce(&OsStr) -> io::Result<File>,
) -> Result<Box<dyn Read>, CliError> {
    if path == "-" {
        return Ok(Box::new(io::stdin().lock()));
    }
    match open(path) {
        Ok(file) => Ok(Box::new(file)),
        Err(e) => Err(CliError::io(format!("cannot open {}", describe(path)), e)),
    }
}

/// Open `path` for writing, truncating it; `-` is standard output.
pub fn open_output(path: &OsStr) -> Result<Box<dyn Write>, CliError> {
    if path == "-" {
        return Ok(Box::new(io::stdout().lock()));
    }
    match File::create(path) {
        Ok(file) => Ok(Box::new(file)),
        Err(e) => Err(CliError::io(format!("cannot create {}", describe(path)), e)),
    }
}

/// Read `reader` to the end in [`BUFFER`]-sized chunks, passing each to
/// `chunk`. `chunk` returns `false` to stop early. Interrupted reads retry.
pub fn feed(reader: &mut dyn Read, mut chunk: impl FnMut(&[u8]) -> bool) -> io::Result<()> {
    let mut buf = vec![0u8; BUFFER];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                if !chunk(&buf[..n]) {
                    return Ok(());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// Write `text` to standard output and flush it.
pub fn print_stdout(text: &str) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(write_error)
}

/// The error for a failed write to standard output.
pub fn write_error(source: io::Error) -> CliError {
    CliError::io("cannot write to standard output", source)
}

/// Print a diagnostic on standard error, ignoring a closed stderr.
pub fn note(message: &str) {
    let _ = writeln!(io::stderr().lock(), "gource-rs: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn non_unicode_input_reaches_the_opener_unchanged_and_reads_the_file() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = OsString::from_vec(b"odd-\xff\xfe.log".to_vec());
        let crate::args::Parsed::Args(args) =
            crate::args::parse("convert", std::slice::from_ref(&path), &[]).unwrap()
        else {
            panic!("a file path is not help");
        };
        let [input] = args.positionals(["<in>"]).unwrap();
        let mut reader = open_input_with(input, |received| {
            assert_eq!(received.as_bytes(), path.as_bytes());
            // A real file handle, not a mocked reader. APFS cannot store the
            // raw-byte name, so only the filesystem lookup is substituted.
            File::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/custom-log/single-event.log"
            ))
        })
        .unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            include_bytes!("../../../tests/fixtures/custom-log/single-event.log")
        );
    }

    #[test]
    fn input_open_failure_preserves_the_path_and_io_error() {
        let error = match open_input_with(OsStr::new("unreadable.log"), |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }) {
            Err(error) => error,
            Ok(_) => panic!("a failed open must not return a reader"),
        };
        assert_eq!(error.exit_code(), 1);
        let CliError::Io { context, source } = error else {
            panic!("an open failure must stay an I/O error");
        };
        assert_eq!(context, "cannot open 'unreadable.log'");
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn standard_input_does_not_call_the_file_opener() {
        assert!(open_input_with(OsStr::new("-"), |_| panic!("stdin is not a file")).is_ok());
    }

    /// Returns data in odd-sized pieces, with an `Interrupted` in between.
    struct Dribble<'a> {
        data: &'a [u8],
        interrupted: bool,
    }

    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let n = self.data.len().min(buf.len()).min(3);
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    #[test]
    fn feed_reads_everything_and_retries_interrupts() {
        let mut reader = Dribble {
            data: b"hello, world",
            interrupted: false,
        };
        let mut seen = Vec::new();
        feed(&mut reader, |c| {
            seen.extend_from_slice(c);
            true
        })
        .unwrap();
        assert_eq!(seen, b"hello, world");
    }

    #[test]
    fn feed_stops_when_asked() {
        let mut reader = Dribble {
            data: b"abcdefghij",
            interrupted: true,
        };
        let mut seen = Vec::new();
        feed(&mut reader, |c| {
            seen.extend_from_slice(c);
            false
        })
        .unwrap();
        assert_eq!(seen, b"abc");
    }

    #[test]
    fn describe_names_stdin() {
        assert_eq!(describe(OsStr::new("-")), "standard input");
        assert_eq!(describe(OsStr::new("a/b.log")), "'a/b.log'");
    }
}
