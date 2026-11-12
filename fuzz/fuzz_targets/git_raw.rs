// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

use gource_sources::{CustomLogWriter, GitRawParser};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let parse = |chunk_size: usize| {
        let mut parser = GitRawParser::with_commit_limits(4096, 1024 * 1024);
        let mut writer = CustomLogWriter::new(Vec::new());
        for chunk in data.chunks(chunk_size) {
            parser.push(chunk, &mut writer);
        }
        parser.finish(&mut writer);
        (writer.finish().unwrap(), parser.stats(), parser.limit_exceeded())
    };
    let chunk_size = data.first().copied().unwrap_or(0) as usize + 1;
    assert_eq!(parse(data.len().max(1)), parse(chunk_size));
});
