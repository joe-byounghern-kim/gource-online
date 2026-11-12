// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

mod custom_log_property;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| custom_log_property::check(data));
