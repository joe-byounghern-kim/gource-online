// SPDX-License-Identifier: GPL-3.0-or-later

//! Where a recorder's event log comes from (`recording.md` section 5.4).
//!
//! On the main thread the fork clones the live engine's [`EventLog`] in
//! wasm memory: same interned strings, commits, attribution sets, total
//! bytes and completeness, with no serialisation and no second parse. A
//! worker export (a later milestone) has its own wasm instance and adds a
//! second implementation that rebuilds the log from encoded batches, so the
//! recorder's constructors do not change.

use gource_core::events::EventLog;
use gource_render::Backend;

use crate::engine::{Clock, Engine};

/// A source of the event log to record.
pub trait LogSource {
    /// A complete, independent copy of the log.
    fn clone_log(&self) -> EventLog;
}

impl LogSource for EventLog {
    fn clone_log(&self) -> EventLog {
        self.clone()
    }
}

impl<B: Backend, C: Clock> LogSource for Engine<B, C> {
    fn clone_log(&self) -> EventLog {
        self.log().clone()
    }
}
