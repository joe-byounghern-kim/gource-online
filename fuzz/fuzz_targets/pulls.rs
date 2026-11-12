// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

use gource_sources::pulls::{events, parse_lines};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(pulls) = parse_lines(text)
    {
        let all = events(&pulls, None, None);
        assert!(all.windows(2).all(|pair| pair[0].timestamp <= pair[1].timestamp));
        for event in all {
            assert!(event.path.starts_with("/pull requests/#"));
            assert!(!event.path.chars().any(char::is_control));
        }
    }
});
