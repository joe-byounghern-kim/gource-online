// SPDX-License-Identifier: GPL-3.0-or-later

//! The little JSON that `bench --json` needs: objects of numbers, strings
//! and nested objects, written with two-space indentation.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// A number, already formatted. Only built through the constructors
    /// below, which only produce valid JSON number syntax (or `null`).
    Raw(String),
    Str(String),
    /// Members in output order.
    Object(Vec<(&'static str, Json)>),
}

impl Json {
    pub fn uint(value: u64) -> Json {
        Json::Raw(value.to_string())
    }

    pub fn int(value: i64) -> Json {
        Json::Raw(value.to_string())
    }

    /// Milliseconds with nanosecond resolution.
    pub fn millis(value: f64) -> Json {
        if value.is_finite() {
            Json::Raw(format!("{value:.6}"))
        } else {
            Json::Raw("null".to_string())
        }
    }

    /// An `f32` in its shortest round-trip form.
    pub fn float(value: f32) -> Json {
        if value.is_finite() {
            Json::Raw(value.to_string())
        } else {
            Json::Raw("null".to_string())
        }
    }

    pub fn string(value: impl Into<String>) -> Json {
        Json::Str(value.into())
    }

    /// The document with a trailing newline.
    pub fn to_pretty_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Raw(text) => out.push_str(text),
            Json::Str(text) => write_string(out, text),
            Json::Object(members) if members.is_empty() => out.push_str("{}"),
            Json::Object(members) => {
                out.push_str("{\n");
                for (i, (key, value)) in members.iter().enumerate() {
                    indent(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                    out.push_str(if i + 1 < members.len() { ",\n" } else { "\n" });
                }
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_objects_are_indented() {
        let doc = Json::Object(vec![
            ("a", Json::uint(1)),
            (
                "b",
                Json::Object(vec![("c", Json::string("x")), ("d", Json::int(-2))]),
            ),
            ("e", Json::Object(Vec::new())),
        ]);
        assert_eq!(
            doc.to_pretty_string(),
            "{\n  \"a\": 1,\n  \"b\": {\n    \"c\": \"x\",\n    \"d\": -2\n  },\n  \"e\": {}\n}\n"
        );
    }

    #[test]
    fn strings_are_escaped() {
        let doc = Json::Object(vec![(
            "p",
            Json::string("a\"b\\c\nd\te\u{1}f\u{e9}\u{4e2d}"),
        )]);
        assert_eq!(
            doc.to_pretty_string(),
            "{\n  \"p\": \"a\\\"b\\\\c\\nd\\te\\u0001f\u{e9}\u{4e2d}\"\n}\n"
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(Json::millis(1.5), Json::Raw("1.500000".into()));
        assert_eq!(Json::millis(0.0), Json::Raw("0.000000".into()));
        assert_eq!(Json::millis(f64::NAN), Json::Raw("null".into()));
        assert_eq!(Json::millis(f64::INFINITY), Json::Raw("null".into()));
        assert_eq!(Json::float(0.5), Json::Raw("0.5".into()));
        assert_eq!(Json::float(1.0 / 60.0), Json::Raw("0.016666668".into()));
        assert_eq!(Json::float(f32::INFINITY), Json::Raw("null".into()));
        assert_eq!(
            Json::uint(u64::MAX),
            Json::Raw("18446744073709551615".into())
        );
    }
}
