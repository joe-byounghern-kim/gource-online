// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `RCommitLog::filter_utf8` (`formats/commitlog.cpp`).
//!
//! Gource runs usernames and file names through
//! `utf8::replace_invalid(begin, end, out, '?')` from the bundled utf8cpp
//! (`core/utf8/`). That is *not* what `String::from_utf8_lossy` does: the
//! replacement is a single `?` rather than U+FFFD, and the number of bytes
//! swallowed per replacement follows utf8cpp's rules:
//!
//! - a byte that cannot start a sequence (`0x80..=0xBF`, `0xF8..=0xFF`) is
//!   replaced on its own, so three stray continuation bytes give `???`;
//! - a sequence that is cut short by a non-continuation byte, is overlong,
//!   or decodes to a surrogate or a value above U+10FFFF is replaced once,
//!   and every continuation byte that follows it is dropped with it;
//! - a sequence cut short by the end of the input is replaced once and the
//!   rest of the input is dropped.
//!
//! Valid UTF-8 is copied unchanged, and the set of sequences utf8cpp accepts
//! equals what `std::str::from_utf8` accepts, so that is the fast path.

/// Why `validate_next` rejected the sequence at the cursor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Reject {
    /// The input ended inside the sequence.
    NotEnoughRoom,
    /// The lead byte cannot start a sequence.
    InvalidLead,
    /// A byte that should have been a continuation was not.
    Incomplete,
    /// Encoded in more bytes than needed.
    Overlong,
    /// Surrogate or above U+10FFFF.
    InvalidCodePoint,
}

fn is_trail(b: u8) -> bool {
    b >> 6 == 0b10
}

/// `utf8::internal::validate_next`: decode one sequence from the start of
/// `b` (non-empty) and return `(code point, length)`.
fn validate_next(b: &[u8]) -> Result<(u32, usize), Reject> {
    let lead = b[0];
    let (length, mut cp) = if lead < 0x80 {
        (1, u32::from(lead))
    } else if lead >> 5 == 0b110 {
        (2, (u32::from(lead) << 6) & 0x7ff)
    } else if lead >> 4 == 0b1110 {
        (3, (u32::from(lead) << 12) & 0xffff)
    } else if lead >> 3 == 0b11110 {
        (4, (u32::from(lead) << 18) & 0x001f_ffff)
    } else {
        return Err(Reject::InvalidLead);
    };

    for k in 1..length {
        let Some(&t) = b.get(k) else {
            return Err(Reject::NotEnoughRoom);
        };
        if !is_trail(t) {
            return Err(Reject::Incomplete);
        }
        cp += u32::from(t & 0x3f) << (6 * (length - 1 - k));
    }

    if cp > 0x0010_ffff || (0xd800..=0xdfff).contains(&cp) {
        return Err(Reject::InvalidCodePoint);
    }
    let overlong = match cp {
        0..=0x7f => length != 1,
        0x80..=0x7ff => length != 2,
        0x800..=0xffff => length != 3,
        _ => false,
    };
    if overlong {
        return Err(Reject::Overlong);
    }
    Ok((cp, length))
}

/// Append the filtered form of `input` to `out`. Never fails.
pub(crate) fn filter_utf8_into(input: &[u8], out: &mut String) {
    if let Ok(s) = std::str::from_utf8(input) {
        out.push_str(s);
        return;
    }

    let mut i = 0;
    while i < input.len() {
        match validate_next(&input[i..]) {
            Ok((cp, length)) => {
                out.push(char::from_u32(cp).unwrap_or('?'));
                i += length;
            }
            Err(Reject::NotEnoughRoom) => {
                out.push('?');
                i = input.len();
            }
            Err(Reject::InvalidLead) => {
                out.push('?');
                i += 1;
            }
            Err(Reject::Incomplete | Reject::Overlong | Reject::InvalidCodePoint) => {
                out.push('?');
                i += 1;
                // one replacement for the whole broken sequence
                while i < input.len() && is_trail(input[i]) {
                    i += 1;
                }
            }
        }
    }
}

/// The filtered form of `input`, borrowed when it is already valid UTF-8.
/// `scratch` holds the result otherwise.
pub(crate) fn filter_utf8<'a>(input: &'a [u8], scratch: &'a mut String) -> &'a str {
    match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => {
            scratch.clear();
            filter_utf8_into(input, scratch);
            scratch.as_str()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(input: &[u8]) -> String {
        let mut out = String::new();
        filter_utf8_into(input, &mut out);
        out
    }

    #[test]
    fn valid_input_is_unchanged() {
        assert_eq!(f(b""), "");
        assert_eq!(f(b"plain/ascii.txt"), "plain/ascii.txt");
        let s = "Árvíztűrő 深 ë 🌲 \u{10FFFF}";
        assert_eq!(f(s.as_bytes()), s);
        assert_eq!(f(b"nul\0inside"), "nul\0inside");
    }

    #[test]
    fn stray_continuation_and_invalid_lead_bytes_are_replaced_one_by_one() {
        assert_eq!(f(b"\x80"), "?");
        assert_eq!(f(b"\x80\x80\x80"), "???");
        assert_eq!(f(b"a\xffb\xfec"), "a?b?c");
        assert_eq!(f(b"\xf8\x88\x80\x80\x80"), "?????");
    }

    #[test]
    fn broken_sequences_collapse_with_their_continuation_bytes() {
        // lead followed by a non-continuation byte
        assert_eq!(f(b"\xe2\x82X"), "?X");
        assert_eq!(f(b"\xe2X"), "?X");
        // 4-byte lead, two continuations, then ASCII
        assert_eq!(f(b"\xf0\x90\x80X"), "?X");
        // a lead right after another lead
        assert_eq!(f("\u{20ac}".as_bytes()), "\u{20ac}");
        assert_eq!(f(b"\xe2\xe2\x82\xac"), "?\u{20ac}");
        // overlong forms swallow the whole group
        assert_eq!(f(b"\xc0\x80"), "?");
        assert_eq!(f(b"\xc1\xbf!"), "?!");
        assert_eq!(f(b"\xe0\x80\x80"), "?");
        assert_eq!(f(b"\xf0\x80\x80\x80\x80"), "?");
        // surrogates and out-of-range values
        assert_eq!(f(b"\xed\xa0\x80z"), "?z");
        assert_eq!(f(b"\xf4\x90\x80\x80z"), "?z");
        assert_eq!(f(b"\xf5\x80\x80\x80z"), "?z");
        // extra continuation bytes after a broken sequence are swallowed too
        assert_eq!(f(b"\xc0\x80\x80\x80q"), "?q");
    }

    #[test]
    fn truncated_tail_is_one_replacement_and_the_rest_is_dropped() {
        assert_eq!(f(b"ab\xe2"), "ab?");
        assert_eq!(f(b"ab\xe2\x82"), "ab?");
        assert_eq!(f(b"ab\xf0\x9f\x8c"), "ab?");
        assert_eq!(f(b"\xc3"), "?");
    }

    #[test]
    fn valid_sequences_around_errors_survive() {
        assert_eq!(f("caf\u{e9}\u{ff}x".as_bytes()), "caf\u{e9}\u{ff}x");
        let mut v = "é".as_bytes().to_vec();
        v.push(0xff);
        v.extend_from_slice("🌲".as_bytes());
        assert_eq!(f(&v), "é?🌲");
    }

    #[test]
    fn borrowed_when_valid() {
        let mut scratch = String::new();
        let s = filter_utf8(b"ok", &mut scratch);
        assert_eq!(s, "ok");
        assert!(scratch.is_empty());
        let s = filter_utf8(b"o\xffk", &mut scratch);
        assert_eq!(s, "o?k");
    }

    #[test]
    fn output_is_always_valid_utf8_for_every_short_input() {
        // exhaustive over all 1- and 2-byte inputs, sampled 3-byte inputs
        let mut out = String::new();
        for a in 0..=255u8 {
            out.clear();
            filter_utf8_into(&[a], &mut out);
            for b in 0..=255u8 {
                out.clear();
                filter_utf8_into(&[a, b], &mut out);
            }
        }
        for a in [
            0x00, 0x41, 0x7f, 0x80, 0xbf, 0xc0, 0xc2, 0xdf, 0xe0, 0xed, 0xf0, 0xf4, 0xf5, 0xff,
        ] {
            for b in [0x00, 0x7f, 0x80, 0x9f, 0xa0, 0xbf, 0xc0, 0xff] {
                for c in [0x00, 0x7f, 0x80, 0xbf, 0xc0, 0xff] {
                    out.clear();
                    filter_utf8_into(&[a, b, c], &mut out);
                }
            }
        }
    }
}
