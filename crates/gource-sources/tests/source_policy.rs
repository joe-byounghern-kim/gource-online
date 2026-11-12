// SPDX-License-Identifier: GPL-3.0-or-later

//! The source policy itself. The end-to-end tests stop at earlier gates for most of
//! these values, such as the actor map's label characters.

use gource_sources::source::{has_token_marker, safe_path, safe_path_bytes, safe_text};

#[test]
fn a_path_may_hold_an_at_sign_and_text_may_not() {
    for value in ["a@b", "logo@2x.png", "@types", "person@example.test"] {
        assert!(!safe_text(value), "{value}");
        assert!(safe_path(value), "{value}");
        assert!(safe_path_bytes(value.as_bytes()), "{value}");
    }
    for path in ["node_modules/@types/a.d.ts", "/app/@modal/page.tsx"] {
        assert!(safe_path(path), "{path}");
    }
    assert!(safe_text("opaque-label"));
}

#[test]
fn every_other_character_and_marker_is_refused_in_both() {
    let refused: Vec<String> = [
        '|', '\\', '\u{0000}', '\t', '\u{001b}', '\u{001f}', '\u{007f}',
    ]
    .into_iter()
    .chain(['\u{0080}', '\u{009f}', '\u{061c}', '\u{200e}', '\u{200f}'])
    .chain(['\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}'])
    .chain(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'])
    .map(|c| format!("a{c}b"))
    .chain(
        ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]
            .iter()
            .map(|marker| format!("a{marker}b")),
    )
    .collect();
    for value in &refused {
        assert!(!safe_text(value), "{value:?}");
        assert!(!safe_path(value), "{value:?}");
        // An @ beside it buys nothing.
        assert!(!safe_path(&format!("@{value}")), "@{value:?}");
    }
    assert!(has_token_marker("ghp_"));
    assert!(!has_token_marker("ghx_"));
}

#[test]
fn a_path_refuses_empty_and_relative_components() {
    for path in [
        "", "/", "a//b", "a/", "/a/", ".", "..", "a/./b", "a/../b", "@/../b",
    ] {
        assert!(!safe_path(path), "{path:?}");
    }
    assert!(safe_path("/a/b"));
    assert!(safe_path("a/b"));
    assert!(!safe_path_bytes(b"a@\xffb"));
}
