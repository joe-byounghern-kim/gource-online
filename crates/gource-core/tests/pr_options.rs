// SPDX-License-Identifier: GPL-3.0-or-later
use gource_core::settings::{OptionEffect, Settings};

#[test]
fn rich_controls_round_trip_without_changing_plain_defaults() {
    let mut settings = Settings::default();
    for (name, default) in [
        ("pull-request-style", "dim-highlight"),
        ("pull-request-mode", "basic"),
        ("pr-flow-hud", "false"),
        ("hide-pull-requests", "false"),
    ] {
        assert_eq!(settings.option_value(name).as_deref(), Some(default));
        assert!(Settings::option_names().contains(&name));
        settings.apply_option(name, default).unwrap();
    }
    assert_eq!(settings.option_value("hide").as_deref(), Some(""));
    assert_eq!(
        settings.apply_option("pull-request-mode", "full").unwrap(),
        OptionEffect::Live
    );
    assert_eq!(
        settings.option_value("pull-request-mode").as_deref(),
        Some("full")
    );
    settings.apply_option("pr-flow-hud", "1").unwrap();
    assert_eq!(
        settings.option_value("pr-flow-hud").as_deref(),
        Some("true")
    );
    settings.apply_option("pull-request-mode", "").unwrap();
    assert_eq!(
        settings.option_value("pull-request-mode").as_deref(),
        Some("basic")
    );
}

#[test]
fn only_a_changed_pr_hide_bit_requests_rebuild() {
    let mut settings = Settings::default();
    assert_eq!(
        settings.apply_option("hide", "filenames").unwrap(),
        OptionEffect::Live
    );
    assert_eq!(
        settings
            .apply_option("hide", "filenames,pull-requests")
            .unwrap(),
        OptionEffect::Restart
    );
    assert_eq!(
        settings.option_value("hide").as_deref(),
        Some("filenames,pull-requests")
    );
    assert_eq!(
        settings.apply_option("hide-pull-requests", "true").unwrap(),
        OptionEffect::Live
    );
    assert_eq!(
        settings
            .apply_option("hide-pull-requests", "false")
            .unwrap(),
        OptionEffect::Restart
    );
    assert_eq!(settings.option_value("hide").as_deref(), Some("filenames"));
    settings.apply_option("hide-pull-requests", "true").unwrap();
    assert_eq!(
        settings.apply_option("hide", "").unwrap(),
        OptionEffect::Restart
    );
}

#[test]
fn invalid_styles_modes_and_hide_values_do_not_mutate_settings() {
    let mut settings = Settings::default();
    let before = settings.clone();
    for (name, value) in [
        ("pull-request-style", "ghost-files"),
        ("pull-request-mode", "FULL"),
        ("hide", "pull-requests,unknown"),
        ("pr-flow-hud", "maybe"),
    ] {
        assert!(settings.apply_option(name, value).is_err());
        assert_eq!(settings, before);
    }
}
