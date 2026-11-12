// SPDX-License-Identifier: GPL-3.0-or-later

//! Just enough JSON for the recording API: a strict parser for the small
//! objects the page passes in (option pairs, muxer config, recipe context)
//! and a writer for the objects handed back. The wasm build carries no
//! serialisation crate (see `status.rs`).

use std::fmt::Write;

use crate::status::push_json_string;

/// Inputs are small; deeper nesting is rejected rather than recursed into.
const MAX_DEPTH: usize = 32;

/// A parsed JSON value. Numbers keep their literal text, so an option value
/// given as a number (`"record-framerate": 30`) reaches the option parser
/// exactly as written.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Members in input order; a repeated key is rejected by the parser.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn members(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// A scalar as option text: strings as they are, numbers as written,
    /// booleans as `true`/`false`. `None` for null, arrays and objects.
    pub fn as_option_text(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::Number(n) => Some(n),
            Value::Bool(true) => Some("true"),
            Value::Bool(false) => Some("false"),
            _ => None,
        }
    }

    /// Compact JSON text.
    pub fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(n),
            Value::String(s) => push_json_string(out, s),
            Value::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Object(members) => {
                out.push('{');
                for (i, (k, v)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    push_json_string(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
}

/// A number value from an `f64` (non-finite values become 0, as JSON has
/// no representation for them).
pub fn number(v: f64) -> Value {
    let mut s = String::new();
    push_number(&mut s, v);
    Value::Number(s)
}

/// An object from `(key, value)` pairs.
pub fn object<const N: usize>(members: [(&str, Value); N]) -> Value {
    Value::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

pub fn string(s: &str) -> Value {
    Value::String(s.to_string())
}

/// Shortest text that reads back as the same `f64`; integers without a
/// fraction.
pub fn push_number(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push('0');
    } else if v == v.trunc() && v.abs() < 9.0e15 {
        let _ = write!(out, "{}", v as i64);
    } else {
        let _ = write!(out, "{v}");
    }
}

/// Parse error with the byte offset where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub offset: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.offset, self.reason)
    }
}

/// Parse one JSON document (RFC 8259; whitespace around it allowed).
pub fn parse(text: &str) -> Result<Value, JsonError> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, reason: &'static str) -> JsonError {
        JsonError {
            offset: self.i,
            reason,
        }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.s.get(self.i) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.i += 1;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.eat(b"}") {
            return Ok(Value::Object(members));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.err("expected a key"));
            }
            let at = self.i;
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(JsonError {
                    offset: at,
                    reason: "duplicate key",
                });
            }
            self.ws();
            if !self.eat(b":") {
                return Err(self.err("expected ':'"));
            }
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"}") {
                return Ok(Value::Object(members));
            }
            return Err(self.err("expected ',' or '}'"));
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b"]") {
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"]") {
                return Ok(Value::Array(items));
            }
            return Err(self.err("expected ',' or ']'"));
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while let Some(b'0'..=b'9') = p.s.get(p.i) {
                p.i += 1;
            }
            p.i > from
        };
        self.eat(b"-");
        if self.eat(b"0") {
            // no leading zeros
        } else if !digits(self) {
            return Err(self.err("expected a digit"));
        }
        if self.eat(b".") && !digits(self) {
            return Err(self.err("expected a digit"));
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            self.i += 1;
            if let Some(b'+' | b'-') = self.s.get(self.i) {
                self.i += 1;
            }
            if !digits(self) {
                return Err(self.err("expected a digit"));
            }
        }
        // ASCII only, so the slice is valid UTF-8
        let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("0");
        Ok(Value::Number(text.to_string()))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .s
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(&b) = self.s.get(self.i) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // the input is a &str and the run stops at ASCII bytes only
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).unwrap_or(""));
            match self.s.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let esc = *self
                        .s
                        .get(self.i)
                        .ok_or_else(|| self.err("unexpected end"))?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi) {
                                if !self.eat(b"\\u") {
                                    return Err(self.err("lone surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return Err(self.err("lone surrogate"));
                                }
                                0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(c).ok_or_else(|| self.err("lone surrogate"))?);
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_back() {
        let text = r#"{"a":"x\"y\n\u00e9\ud83d\ude00","b":[1,-2.5e3,true,false,null],"c":{}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.get("a").unwrap().as_str(), Some("x\"y\né😀"));
        assert_eq!(
            v.get("b").unwrap(),
            &Value::Array(vec![
                Value::Number("1".into()),
                Value::Number("-2.5e3".into()),
                Value::Bool(true),
                Value::Bool(false),
                Value::Null,
            ])
        );
        assert_eq!(parse(&v.to_json()).unwrap(), v);
        assert_eq!(v.get("b").unwrap().to_json(), "[1,-2.5e3,true,false,null]");
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "{",
            "{\"a\":1,}",
            "[1,]",
            "01",
            "1.",
            "-",
            "\"\\x\"",
            "\"a",
            "{\"a\":1,\"a\":2}",
            "{\"a\" 1}",
            "[1] 2",
            "\"\\ud800\"",
            "\"\u{1}\"",
            "nul",
            "{1:2}",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
        let deep = "[".repeat(40) + &"]".repeat(40);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn option_text_keeps_numbers_as_written() {
        let v = parse(r#"{"n":30,"f":0.50,"b":true,"s":"1920x1080","z":null}"#).unwrap();
        let text = |k: &str| v.get(k).and_then(Value::as_option_text);
        assert_eq!(text("n"), Some("30"));
        assert_eq!(text("f"), Some("0.50"));
        assert_eq!(text("b"), Some("true"));
        assert_eq!(text("s"), Some("1920x1080"));
        assert_eq!(text("z"), None);
        assert_eq!(number(0.25).to_json(), "0.25");
        assert_eq!(number(3.0).to_json(), "3");
        assert_eq!(number(f64::NAN).to_json(), "0");
    }
}
