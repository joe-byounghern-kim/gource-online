// SPDX-License-Identifier: GPL-3.0-or-later
use gource_sources::{CustomLogParser, CustomLogWriter};

#[path = "../../../fuzz/fuzz_targets/custom_log_property.rs"]
mod custom_log_property;

const REJECTED: &[u8] = include_bytes!("../../../fuzz/seeds/custom_log/unsupported-pr-coverage");

#[test]
fn parser_reachable_unknown_coverage_is_a_deliberate_export_rejection() {
    for chunk in 1..=REJECTED.len() {
        let mut parser = CustomLogParser::new();
        let mut writer = CustomLogWriter::new(Vec::new());
        for bytes in REJECTED.chunks(chunk) {
            parser.push(bytes, &mut writer);
        }
        parser.finish(&mut writer);
        let error = writer.finish().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "unsupported frozen PR coverage for rich custom export"
        );
    }
}

#[test]
fn fuzz_property_compares_rejections_instead_of_panicking() {
    custom_log_property::check(REJECTED);
    custom_log_property::check(b"\0\n0|||a||pr-log=");
    custom_log_property::check(b"");
    custom_log_property::check(b"1|author|A|/src/main.rs\n");
}

#[test]
fn rejection_seed_is_byte_deletion_minimized() {
    for index in 0..REJECTED.len() {
        let mut reduced = REJECTED.to_vec();
        reduced.remove(index);
        let mut parser = CustomLogParser::new();
        let mut writer = CustomLogWriter::new(Vec::new());
        parser.push(&reduced, &mut writer);
        parser.finish(&mut writer);
        assert!(writer.finish().is_ok(), "byte {index} was not required");
    }
}
