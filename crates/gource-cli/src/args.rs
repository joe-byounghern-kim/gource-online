// SPDX-License-Identifier: GPL-3.0-or-later

//! Hand-rolled command-line parsing shared by the subcommands.
//!
//! Long options take their value as the next argument or after `=`
//! (`--frames 600`, `--frames=600`); switches take none. `-h` and `--help`
//! stop parsing and ask for the help text. A lone `-` is a positional
//! argument (standard input or output), and everything after `--` is too.
//! Positional arguments stay `OsString`, so paths need not be valid Unicode.

use std::ffi::{OsStr, OsString};
use std::ops::RangeInclusive;

use crate::error::CliError;

/// One accepted long option.
#[derive(Copy, Clone, Debug)]
pub struct OptionSpec {
    /// With the leading dashes, e.g. `--frames`.
    pub name: &'static str,
    /// `--name <value>` when true, a switch when false.
    pub takes_value: bool,
}

impl OptionSpec {
    pub const fn value(name: &'static str) -> Self {
        OptionSpec {
            name,
            takes_value: true,
        }
    }

    pub const fn switch(name: &'static str) -> Self {
        OptionSpec {
            name,
            takes_value: false,
        }
    }
}

/// Result of [`parse`].
#[derive(Debug)]
pub enum Parsed {
    /// `-h` or `--help` was given.
    Help,
    Args(Args),
}

/// The parsed command line of one subcommand.
#[derive(Debug)]
pub struct Args {
    command: &'static str,
    positionals: Vec<OsString>,
    values: Vec<(&'static str, String)>,
    switches: Vec<&'static str>,
}

/// Parse the arguments after the subcommand name.
pub fn parse(
    command: &'static str,
    args: &[OsString],
    specs: &[OptionSpec],
) -> Result<Parsed, CliError> {
    let usage = |message: String| CliError::usage(command, message);
    let mut out = Args {
        command,
        positionals: Vec::new(),
        values: Vec::new(),
        switches: Vec::new(),
    };

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let bytes = arg.as_encoded_bytes();
        if bytes == b"--" {
            out.positionals.extend(rest.by_ref().cloned());
            break;
        }
        if bytes.len() < 2 || bytes[0] != b'-' {
            out.positionals.push(arg.clone());
            continue;
        }
        let Some(text) = arg.to_str() else {
            return Err(usage(format!("unknown option '{}'", arg.to_string_lossy())));
        };
        if text == "-h" || text == "--help" {
            return Ok(Parsed::Help);
        }

        let (name, inline) = match text.split_once('=') {
            Some((name, value)) if text.starts_with("--") => (name, Some(value)),
            _ => (text, None),
        };
        let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
            return Err(usage(format!("unknown option '{name}'")));
        };
        if out.is_set(spec.name) {
            return Err(usage(format!("option '{name}' given more than once")));
        }

        if spec.takes_value {
            let value = match inline {
                Some(value) => value.to_string(),
                None => match rest.next() {
                    Some(next) => match next.to_str() {
                        Some(value) => value.to_string(),
                        None => {
                            return Err(usage(format!("value for '{name}' is not valid Unicode")));
                        }
                    },
                    None => return Err(usage(format!("option '{name}' requires a value"))),
                },
            };
            out.values.push((spec.name, value));
        } else {
            if inline.is_some() {
                return Err(usage(format!("option '{name}' does not take a value")));
            }
            out.switches.push(spec.name);
        }
    }
    Ok(Parsed::Args(out))
}

impl Args {
    fn is_set(&self, name: &str) -> bool {
        self.switch(name) || self.value(name).is_some()
    }

    /// The raw value of `--name`, if given.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn switch(&self, name: &str) -> bool {
        self.switches.contains(&name)
    }

    /// `--name` as an integer within `range`; `None` when not given.
    pub fn uint(
        &self,
        name: &'static str,
        range: RangeInclusive<u32>,
    ) -> Result<Option<u32>, CliError> {
        let Some(text) = self.value(name) else {
            return Ok(None);
        };
        match text.parse::<u32>() {
            Ok(n) if range.contains(&n) => Ok(Some(n)),
            _ => Err(self.invalid(
                name,
                text,
                format!(
                    "expected an integer from {} to {}",
                    range.start(),
                    range.end()
                ),
            )),
        }
    }

    /// `--name` as a duration in seconds (see [`parse_seconds`]); `None`
    /// when not given.
    pub fn seconds(&self, name: &'static str) -> Result<Option<f32>, CliError> {
        let Some(text) = self.value(name) else {
            return Ok(None);
        };
        match parse_seconds(text) {
            Some(s) => Ok(Some(s)),
            None => Err(self.invalid(
                name,
                text,
                format!(
                    "expected a number of seconds above 0 and up to {MAX_SECONDS}, \
                     as a decimal or a fraction such as 1/60"
                ),
            )),
        }
    }

    /// Exactly the positional arguments named by `names`, in order.
    pub fn positionals<const N: usize>(&self, names: [&str; N]) -> Result<[&OsStr; N], CliError> {
        if let Some(missing) = names.get(self.positionals.len()) {
            return Err(CliError::usage(
                self.command,
                format!("missing argument {missing}"),
            ));
        }
        if let Some(extra) = self.positionals.get(N) {
            return Err(CliError::usage(
                self.command,
                format!("unexpected argument '{}'", extra.to_string_lossy()),
            ));
        }
        Ok(std::array::from_fn(|i| self.positionals[i].as_os_str()))
    }

    fn invalid(&self, name: &str, value: &str, expected: String) -> CliError {
        CliError::usage(
            self.command,
            format!("invalid value '{value}' for {name}: {expected}"),
        )
    }
}

/// Longest accepted frame time in seconds. Catches unit mistakes.
pub const MAX_SECONDS: u32 = 3600;

/// A positive frame time in seconds: a decimal (`0.05`) or a fraction
/// (`1/60`), at most [`MAX_SECONDS`]. Computed in `f64`, then rounded once to
/// the `f32` the simulation takes.
pub fn parse_seconds(text: &str) -> Option<f32> {
    let value = match text.split_once('/') {
        Some((num, den)) => parse_number(num)? / parse_number(den)?,
        None => parse_number(text)?,
    };
    let seconds = value as f32;
    (seconds.is_finite() && seconds > 0.0 && seconds <= MAX_SECONDS as f32).then_some(seconds)
}

/// A plain decimal: digits with an optional fraction, no sign, no exponent,
/// no `inf` or `nan` (which `f64::from_str` would accept).
fn parse_number(text: &str) -> Option<f64> {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !digits(int) || !digits(frac) || (int.is_empty() && frac.is_empty()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[OptionSpec] = &[
        OptionSpec::value("--frames"),
        OptionSpec::value("--dt"),
        OptionSpec::switch("--json"),
    ];

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn parse_ok(args: &[&str]) -> Args {
        match parse("test", &os(args), SPECS).unwrap() {
            Parsed::Args(a) => a,
            Parsed::Help => panic!("unexpected help"),
        }
    }

    fn parse_err(args: &[&str]) -> String {
        match parse("test", &os(args), SPECS) {
            Err(e @ CliError::Usage { .. }) => e.to_string(),
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    #[test]
    fn options_values_switches_and_positionals() {
        let a = parse_ok(&["in.log", "--frames", "5", "--dt=0.5", "--json", "-"]);
        assert_eq!(a.value("--frames"), Some("5"));
        assert_eq!(a.value("--dt"), Some("0.5"));
        assert_eq!(a.value("--missing"), None);
        assert!(a.switch("--json"));
        assert!(!a.switch("--frames"));
        let [first, second] = a.positionals(["<in>", "<out>"]).unwrap();
        assert_eq!(first, "in.log");
        assert_eq!(second, "-");
    }

    #[test]
    fn double_dash_ends_options() {
        let a = parse_ok(&["--frames", "5", "--", "--json", "-x"]);
        assert!(!a.switch("--json"));
        let [a1, a2] = a.positionals(["<a>", "<b>"]).unwrap();
        assert_eq!(a1, "--json");
        assert_eq!(a2, "-x");
    }

    #[test]
    fn help_is_recognised_anywhere() {
        for args in [&["-h"][..], &["--help"], &["x", "--frames", "1", "--help"]] {
            assert!(matches!(
                parse("test", &os(args), SPECS).unwrap(),
                Parsed::Help
            ));
        }
    }

    #[test]
    fn bad_options_are_usage_errors() {
        assert_eq!(parse_err(&["--nope"]), "unknown option '--nope'");
        assert_eq!(parse_err(&["--nope=1"]), "unknown option '--nope'");
        assert_eq!(parse_err(&["-x"]), "unknown option '-x'");
        assert_eq!(
            parse_err(&["--frames"]),
            "option '--frames' requires a value"
        );
        assert_eq!(
            parse_err(&["--json=1"]),
            "option '--json' does not take a value"
        );
        assert_eq!(
            parse_err(&["--frames", "1", "--frames=2"]),
            "option '--frames' given more than once"
        );
        assert_eq!(
            parse_err(&["--json", "--json"]),
            "option '--json' given more than once"
        );
    }

    #[test]
    fn positional_count_is_checked() {
        let a = parse_ok(&["one"]);
        assert_eq!(
            a.positionals(["<one>", "<two>"]).unwrap_err().to_string(),
            "missing argument <two>"
        );
        assert_eq!(
            a.positionals::<0>([]).unwrap_err().to_string(),
            "unexpected argument 'one'"
        );
        assert!(a.positionals(["<one>"]).is_ok());
    }

    #[test]
    fn integers_are_range_checked() {
        let uint = |value: &str| {
            parse_ok(&["--frames", value])
                .uint("--frames", 1..=10)
                .map_err(|e| e.to_string())
        };
        assert_eq!(uint("7"), Ok(Some(7)));
        assert_eq!(uint("10"), Ok(Some(10)));
        for bad in ["0", "11", "-1", "abc", "", "1.5", " 1"] {
            let err = uint(bad).unwrap_err();
            assert!(
                err.starts_with(&format!(
                    "invalid value '{bad}' for --frames: expected an integer from 1 to 10"
                )),
                "{err}"
            );
        }
        assert_eq!(parse_ok(&[]).uint("--frames", 1..=10).unwrap(), None);
    }

    #[test]
    fn seconds_accept_decimals_and_fractions() {
        assert_eq!(parse_seconds("1/60"), Some(1.0 / 60.0));
        assert_eq!(parse_seconds("0.5"), Some(0.5));
        assert_eq!(parse_seconds("2"), Some(2.0));
        assert_eq!(parse_seconds(".25"), Some(0.25));
        assert_eq!(parse_seconds("1/30"), Some(1.0 / 30.0));
        assert_eq!(parse_seconds("3600"), Some(3600.0));
        for bad in [
            "", "0", "0.0", "-1", "+1", "1/0", "0/5", "1/", "/60", "1/2/3", "inf", "nan", "1e-3",
            "3601", "abc", "1.2.3", ".", "1e400",
        ] {
            assert_eq!(parse_seconds(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_default_frame_time_is_the_f32_nearest_to_a_sixtieth() {
        assert_eq!(parse_seconds("1/60"), Some(1.0_f32 / 60.0_f32));
    }
}
