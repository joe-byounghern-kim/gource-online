// SPDX-License-Identifier: GPL-3.0-or-later

//! Narrow opt-in source grammar. These checks are NOT a universal secret scanner
//! or proof that a supplied SHA belongs to a repository. Some valid Git filenames
//! are deliberately refused. Ordinary parsers and writers do not use this policy.

/// Full lowercase SHA-1 or SHA-256 object identity, never an abbreviation.
pub fn valid_sha(value: &[u8]) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

/// Reject known GitHub credential markers wherever they occur in emitted fields.
/// Arbitrary secrets without these markers cannot be detected by this denylist.
pub fn has_token_marker(value: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]
        .iter()
        .any(|marker| value.contains(marker))
}

/// Delimiters, backslashes, known token markers and the fixed C0/DEL/C1 plus
/// U+061C, U+200E/F, U+202A-E, U+2066-9 control/bidi set, in any source field.
fn safe_field(value: &str) -> bool {
    !value.contains(['|', '\\']) && !has_token_marker(value)
        && !value.chars().any(|c| matches!(c, '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' | '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}

/// Labels and attribution: `safe_field`, and no email-like text.
pub fn safe_text(value: &str) -> bool {
    !value.contains('@') && safe_field(value)
}

/// Validate DECODED bytes before the git parser performs lossy UTF-8 filtering.
pub fn safe_path_bytes(value: &[u8]) -> bool {
    std::str::from_utf8(value).is_ok_and(safe_path)
}

/// A repository path may hold `@` (`logo@2x.png`, `node_modules/@types`): it is the
/// repository's own file name, not taken from the author field, which source mode
/// replaces with the label. An email-shaped file name is therefore emitted as it is.
pub fn safe_path(value: &str) -> bool {
    let relative = value.strip_prefix('/').unwrap_or(value);
    !relative.is_empty()
        && safe_field(value)
        && relative
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
