// SPDX-License-Identifier: GPL-3.0-or-later

//! Loading a custom log into an [`EventLog`].

use std::ffi::OsStr;
use std::time::{Duration, Instant};

use gource_core::events::EventLog;
use gource_sources::{CustomLogParser, EventLogSink, ParseStats};

use crate::error::CliError;
use crate::streams;

/// A fully loaded log, ready to be played.
pub struct Loaded {
    pub log: EventLog,
    pub stats: ParseStats,
    /// Time spent reading and parsing.
    pub elapsed: Duration,
}

/// Parse the custom log at `path` (`-` is standard input) into an
/// [`EventLog`], then mark it complete with its exact size, as the web host
/// does once the whole log has arrived.
pub fn load_custom_log(path: &OsStr) -> Result<Loaded, CliError> {
    let mut reader = streams::open_input(path)?;
    let started = Instant::now();

    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    {
        let mut sink = EventLogSink::new(&mut log);
        streams::feed(&mut *reader, |chunk| {
            parser.push(chunk, &mut sink);
            true
        })
        .map_err(|e| CliError::io(format!("cannot read {}", streams::describe(path)), e))?;
        parser.finish(&mut sink);
    }

    let stats = parser.stats();
    log.set_total_bytes(stats.bytes);
    log.set_complete(true);
    Ok(Loaded {
        log,
        stats,
        elapsed: started.elapsed(),
    })
}
