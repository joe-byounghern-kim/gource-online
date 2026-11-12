// SPDX-License-Identifier: GPL-3.0-or-later
use gource_sources::{CustomLogParser, CustomLogWriter};

pub fn check(data: &[u8]) {
    let parse = |chunk_size: usize| {
        let mut parser = CustomLogParser::new();
        let mut writer = CustomLogWriter::new(Vec::new());
        for chunk in data.chunks(chunk_size) {
            parser.push(chunk, &mut writer);
        }
        parser.finish(&mut writer);
        let output = writer
            .finish()
            .map_err(|error| (error.kind(), error.to_string()));
        (output, parser.stats())
    };
    let chunk_size = data.first().copied().unwrap_or(0) as usize + 1;
    assert_eq!(parse(data.len().max(1)), parse(chunk_size));
}
