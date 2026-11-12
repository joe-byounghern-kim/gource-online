// SPDX-License-Identifier: GPL-3.0-or-later

//! Errors that end a command early, and the exit status they map to.

use std::fmt;
use std::io;

/// Why a command stopped.
#[derive(Debug)]
pub enum CliError {
    /// The command line is wrong (exit status 2). `command` names the
    /// subcommand whose help applies; `None` means the top-level help.
    Usage {
        command: Option<&'static str>,
        message: String,
    },
    /// Something failed while running (exit status 1).
    Failure(String),
    /// An I/O error together with the operation that hit it (exit status 1).
    Io { context: String, source: io::Error },
    /// An input limit stopped the conversion (exit status 3). What was
    /// written before it stays written.
    Limit { what: &'static str, limit: u64 },
}

impl CliError {
    /// A bad command line for subcommand `command`.
    pub fn usage(command: &'static str, message: impl Into<String>) -> Self {
        CliError::Usage {
            command: Some(command),
            message: message.into(),
        }
    }

    /// A bad command line before any subcommand was recognised.
    pub fn usage_top(message: impl Into<String>) -> Self {
        CliError::Usage {
            command: None,
            message: message.into(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        CliError::Failure(message.into())
    }

    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        CliError::Io {
            context: context.into(),
            source,
        }
    }

    /// Process exit status: 2 for usage errors, 3 for a limit, 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Usage { .. } => 2,
            CliError::Limit { .. } => 3,
            CliError::Failure(_) | CliError::Io { .. } => 1,
        }
    }

    /// The reader of our standard output went away (`gource-rs ... | head`).
    /// That is not a failure worth reporting.
    pub fn is_broken_pipe(&self) -> bool {
        matches!(self, CliError::Io { source, .. } if source.kind() == io::ErrorKind::BrokenPipe)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage { message, .. } | CliError::Failure(message) => f.write_str(message),
            CliError::Io { context, source } => write!(f, "{context}: {source}"),
            CliError::Limit { what, limit } => write!(f, "limit exceeded: {what} {limit}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_kind_of_error() {
        assert_eq!(CliError::usage("bench", "bad").exit_code(), 2);
        assert_eq!(CliError::usage_top("bad").exit_code(), 2);
        assert_eq!(CliError::failure("bad").exit_code(), 1);
        let io = CliError::io("cannot open 'x'", io::Error::other("nope"));
        assert_eq!(io.exit_code(), 1);
        assert_eq!(io.to_string(), "cannot open 'x': nope");
    }

    #[test]
    fn broken_pipe_is_recognised() {
        let pipe = CliError::io("write", io::Error::from(io::ErrorKind::BrokenPipe));
        assert!(pipe.is_broken_pipe());
        assert!(!CliError::io("write", io::Error::other("x")).is_broken_pipe());
        assert!(!CliError::failure("x").is_broken_pipe());
    }
}
