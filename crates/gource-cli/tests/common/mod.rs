// SPDX-License-Identifier: GPL-3.0-or-later

//! Helpers shared by the integration tests: run the built `gource-rs`
//! binary, scratch files, fixtures, and a strict JSON parser (the crate has
//! no dependencies, so the tests bring their own).

#![allow(dead_code)]

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a run of the binary produced.
#[derive(Debug)]
pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// Panics with the captured streams unless the run exited with 0.
    pub fn ok(self) -> Run {
        assert_eq!(self.code, Some(0), "expected success:\n{self:#?}");
        self
    }

    /// Panics unless the run exited with `code` and `stderr` contains
    /// `needle`; also checks that nothing was written to stdout.
    pub fn fails(self, code: i32, needle: &str) -> Run {
        assert_eq!(self.code, Some(code), "wrong exit status:\n{self:#?}");
        assert!(
            self.stderr.contains(needle),
            "stderr lacks {needle:?}:\n{self:#?}"
        );
        assert!(
            self.stdout.is_empty(),
            "stdout should be empty on failure:\n{self:#?}"
        );
        self
    }
}

fn command<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gource-rs"));
    cmd.args(args);
    cmd
}

fn collect(output: std::process::Output) -> Run {
    Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

/// Run `gource-rs args...` with an empty standard input.
pub fn run<I, S>(args: I) -> Run
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = command(args)
        .stdin(Stdio::null())
        .output()
        .expect("run gource-rs");
    collect(output)
}

/// Run `gource-rs args...` with `input` on standard input.
pub fn run_with_stdin<I, S>(args: I, input: &[u8]) -> Run
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = command(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gource-rs");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = input.to_vec();
    // Feed from a thread so a large input cannot deadlock against the
    // child's output.
    let feeder = std::thread::spawn(move || {
        // The child may exit before reading everything (usage errors).
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().expect("wait for gource-rs");
    feeder.join().expect("feeder thread");
    collect(output)
}

/// A custom-log fixture from `tests/fixtures/custom-log/`.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/custom-log")
        .join(name)
}

/// Every `*.log` fixture, sorted by name.
pub fn fixtures() -> Vec<PathBuf> {
    let dir = fixture("");
    let mut logs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
        .collect();
    logs.sort();
    assert!(logs.len() >= 10, "fixtures went missing from {dir:?}");
    logs
}

/// An empty scratch directory private to one test.
pub fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("gource-cli")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

/// Write `contents` to `dir/name` and return the path.
pub fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write scratch file");
    path
}

pub fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).expect("read file")).expect("file is UTF-8")
}

/// The `key=value` fields of one `simulate` report line.
pub fn fields(line: &str) -> Vec<(&str, &str)> {
    line.split(' ')
        .map(|field| field.split_once('=').expect("key=value field"))
        .collect()
}

/// The value of `key` in a `simulate` report line.
pub fn field<'a>(line: &'a str, key: &str) -> &'a str {
    fields(line)
        .into_iter()
        .find(|(k, _)| *k == key)
        .unwrap_or_else(|| panic!("no {key} in {line:?}"))
        .1
}

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Strictly parse one JSON document (RFC 8259): no trailing text, no
    /// duplicate keys, no raw control characters in strings.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        p.skip_ws();
        let value = p.value()?;
        p.skip_ws();
        if p.i != p.s.len() {
            return Err(format!("trailing text at byte {}", p.i));
        }
        Ok(value)
    }

    /// The member at a dotted path (`phases.update.mean_ms`); `""` is the value itself.
    pub fn get(&self, path: &str) -> Option<&Json> {
        if path.is_empty() {
            return Some(self);
        }
        path.split('.').try_fold(self, |value, key| match value {
            Json::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        })
    }

    pub fn num(&self, path: &str) -> f64 {
        match self.get(path) {
            Some(Json::Num(n)) => *n,
            other => panic!("{path} is not a number: {other:?}"),
        }
    }

    pub fn str(&self, path: &str) -> &str {
        match self.get(path) {
            Some(Json::Str(s)) => s,
            other => panic!("{path} is not a string: {other:?}"),
        }
    }

    /// Member names in document order.
    pub fn keys(&self, path: &str) -> Vec<&str> {
        match self.get(path) {
            Some(Json::Object(members)) => members.iter().map(|(k, _)| k.as_str()).collect(),
            other => panic!("{path} is not an object: {other:?}"),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at byte {}", byte as char, self.i))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(value)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            other => Err(format!("unexpected {other:?} at byte {}", self.i)),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            members.push((key, self.value()?));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                return Err("unterminated string".into());
            };
            self.i += 1;
            match b {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                0..=0x1f => return Err(format!("raw control character {b:#04x} in string")),
                b'\\' => {
                    let Some(e) = self.peek() else {
                        return Err("unterminated escape".into());
                    };
                    self.i += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        other => return Err(format!("bad escape \\{}", other as char)),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(b),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.i..self.i + 4).ok_or("short \\u escape")?;
        let text = std::str::from_utf8(digits).map_err(|e| e.to_string())?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("bad \\u escape {text:?}"));
        }
        self.i += 4;
        u32::from_str_radix(text, 16).map_err(|e| e.to_string())
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&first) {
            if !self.s[self.i..].starts_with(b"\\u") {
                return Err("lone high surrogate".into());
            }
            self.i += 2;
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err("bad low surrogate".into());
            }
            0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| format!("bad code point {code:#x}"))
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while matches!(p.peek(), Some(b'0'..=b'9')) {
                p.i += 1;
            }
            p.i - from
        };
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(format!("bad number at byte {start}")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return Err(format!("bad fraction at byte {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return Err(format!("bad exponent at byte {start}"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
        text.parse()
            .map(Json::Num)
            .map_err(|e| format!("{text}: {e}"))
    }
}
