//! The exit-code table and the CLI's typed error.
//!
//! | Code | Class | Source |
//! |------|-------|--------|
//! | 0 | success | - |
//! | 1 | generic infrastructure | bus transport, helper spawn, I/O, config |
//! | 2 | usage | clap rejections, grammar validation, unconfigured `--upload` |
//! | 3 | user-cancelled | capture aborted in the overlay/editor (executor seam) |
//! | 4 | capture-backend | `flowshot_capture::CaptureError` (executor seam) |
//! | 5 | permission denied | portal/protocol permission refusal (executor seam) |
//! | 6 | action-export | `flowshot_actions` export failures incl. the unwritable-dir class (Oracle r4 F-5.ii; executor seam) |
//!
//! Codes 3-6 are produced by the execution wiring:
//! [`exec_exit_code`] maps the executor's outcome/error onto this table.

use std::ffi::OsString;
use std::process::ExitCode;

use flowshot_daemon::DaemonError;
use flowshot_daemon::execute::{ExecOutcome, ExecuteError};

use crate::strings;

/// Clean success.
pub const OK: u8 = 0;
/// Generic infrastructure failure (the anyhow top-level fallback).
pub const GENERIC: u8 = 1;
/// Usage error (clap's own exit code for rejections).
pub const USAGE: u8 = 2;
/// The user cancelled the capture.
pub const CANCELLED: u8 = 3;
/// The capture backend failed.
pub const CAPTURE_BACKEND: u8 = 4;
/// Permission was denied (portal or protocol).
pub const PERMISSION_DENIED: u8 = 5;
/// A post-capture export action failed.
pub const ACTION_EXPORT: u8 = 6;

/// Everything that can fail in the CLI library.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The command line violates the surface grammar or a usage-class rule.
    #[error("{0}")]
    Usage(String),

    /// An argv element cannot cross the `Invoke(as)` wire.
    #[error("{}: {:?}", strings::NON_UNICODE_ARG, .0)]
    NonUnicodeArg(OsString),

    /// A `D-Bus` transport or method call failed.
    #[error("D-Bus failure: {0}")]
    Dbus(#[from] zbus::Error),

    /// The daemon-side handshake (connect/ownership/forward) failed.
    #[error(transparent)]
    Daemon(#[from] DaemonError),

    /// The helper daemon process could not be spawned.
    #[error("could not spawn the FlowShot daemon helper: {0}")]
    Spawn(#[source] std::io::Error),

    /// A local I/O operation failed (writing a generated artifact).
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),

    /// The spawned helper never acquired the bus name within the budget.
    #[error("{} ({detail})", strings::DAEMON_SPAWN_TIMEOUT)]
    DaemonUnavailable {
        /// What was attempted.
        detail: String,
    },
}

/// Maps one executor result onto the exit-code table: success 0,
/// user-cancelled 3, permission-denied 5, capture-backend 4, export 6,
/// usage 2, everything else generic 1.
#[must_use]
pub fn exec_exit_code(result: &Result<ExecOutcome, ExecuteError>) -> ExitCode {
    ExitCode::from(exec_exit_u8(result))
}

/// The [`exec_exit_code`] mapping as the raw table value (tests assert on
/// this - `ExitCode` is deliberately opaque). The error leg is also the
/// one-shot CLI's telemetry seam: the executor's typed failure is
/// captured here (the daemon-forwarded path captures daemon-side in
/// `flowshot_daemon::execute` instead; this mapping only runs for the
/// in-process one-shot results).
#[must_use]
pub fn exec_exit_u8(result: &Result<ExecOutcome, ExecuteError>) -> u8 {
    match result {
        Ok(ExecOutcome::Done(_) | ExecOutcome::ColorPicked(_)) => OK,
        Ok(ExecOutcome::Cancelled) => CANCELLED,
        Err(error) => {
            flowshot_daemon::execute::capture_failure(error);
            exec_error_code(error)
        }
    }
}

fn exec_error_code(error: &ExecuteError) -> u8 {
    match error {
        ExecuteError::Usage(_) => USAGE,
        ExecuteError::Capture(_) if is_permission_denied(error) => PERMISSION_DENIED,
        ExecuteError::Capture(_) | ExecuteError::Probe(_) | ExecuteError::Connect(_) => {
            CAPTURE_BACKEND
        }
        ExecuteError::Export(_) | ExecuteError::Clipboard(_) => ACTION_EXPORT,
        ExecuteError::Child { exit_code, .. } => *exit_code,
        ExecuteError::Ui(_)
        | ExecuteError::Io(_)
        | ExecuteError::Task(_)
        | ExecuteError::Daemon(_) => GENERIC,
    }
}

/// Walks the capture error's source chain for the protocol permission
/// refusal (the denial mapping: compositor denial frame ->
/// `IccError::PermissionDenied` inside `CaptureError::Backend`).
fn is_permission_denied(error: &ExecuteError) -> bool {
    let ExecuteError::Capture(capture) = error else {
        return false;
    };
    let mut source = std::error::Error::source(capture);
    while let Some(cause) = source {
        if let Some(icc) = cause.downcast_ref::<flowshot_capture_wayland::IccError>() {
            return matches!(icc, flowshot_capture_wayland::IccError::PermissionDenied);
        }
        source = cause.source();
    }
    false
}

/// Maps a typed CLI error onto the exit-code table.
#[must_use]
pub const fn exit_code(error: &CliError) -> u8 {
    match error {
        // A vardict the daemon rejected was malformed at the boundary:
        // usage class (the CLI built it, so this is also a bug signal).
        CliError::Usage(_)
        | CliError::NonUnicodeArg(_)
        | CliError::Daemon(DaemonError::InvalidArgs(_)) => USAGE,
        // Executor seam: CANCELLED (overlay abort), CAPTURE_BACKEND
        // (CaptureError), PERMISSION_DENIED (portal/protocol refusal), and
        // ACTION_EXPORT (flowshot_actions failures) map here.
        CliError::Dbus(_)
        | CliError::Daemon(_)
        | CliError::Spawn(_)
        | CliError::Io(_)
        | CliError::DaemonUnavailable { .. } => GENERIC,
    }
}

/// Did-you-mean hints for the rejected legacy Flameshot verbs (
/// `gui`/`launcher`/`screen` exit 2 with a capture hint).
#[must_use]
pub fn legacy_hint(verb: &str) -> Option<&'static str> {
    match verb {
        "gui" => Some(strings::HINT_LEGACY_GUI),
        "launcher" => Some(strings::HINT_LEGACY_LAUNCHER),
        "screen" => Some(strings::HINT_LEGACY_SCREEN),
        _ => None,
    }
}

/// Turns a clap rejection into the process exit: legacy verbs get the
/// friendly hint, everything else clap's own rendering (errors -> stderr
/// exit 2; --help/--version -> stdout exit 0).
#[must_use]
#[expect(
    clippy::print_stderr,
    reason = "the binary's usage-error boundary reports to stderr by contract"
)]
pub fn clap_exit(error: &clap::Error) -> ExitCode {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if error.kind() == ErrorKind::InvalidSubcommand
        && let Some(ContextValue::String(verb)) = error.get(ContextKind::InvalidSubcommand)
        && let Some(hint) = legacy_hint(verb)
    {
        eprintln!("{}", strings::LEGACY_VERB_REJECTED.replace("{0}", verb));
        eprintln!("{hint}");
        return ExitCode::from(USAGE);
    }
    let _ = error.print();
    ExitCode::from(if error.use_stderr() { USAGE } else { OK })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_class_errors_map_to_code_2() {
        assert_eq!(exit_code(&CliError::Usage("x".to_owned())), USAGE);
        assert_eq!(
            exit_code(&CliError::NonUnicodeArg(OsString::from("x"))),
            USAGE
        );
        assert_eq!(
            exit_code(&CliError::Daemon(DaemonError::InvalidArgs("x".to_owned()))),
            USAGE
        );
    }

    #[test]
    fn infrastructure_errors_map_to_code_1() {
        assert_eq!(
            exit_code(&CliError::Spawn(std::io::Error::other("gone"))),
            GENERIC
        );
        assert_eq!(
            exit_code(&CliError::DaemonUnavailable {
                detail: "x".to_owned()
            }),
            GENERIC
        );
        assert_eq!(
            exit_code(&CliError::Daemon(DaemonError::OwnerVanished)),
            GENERIC
        );
    }

    #[test]
    fn legacy_verbs_get_hints_and_nothing_else_does() {
        assert!(legacy_hint("gui").is_some_and(|hint| hint.contains("capture")));
        assert!(legacy_hint("launcher").is_some_and(|hint| hint.contains("--dialog")));
        assert!(legacy_hint("screen").is_some_and(|hint| hint.contains("capture screen")));
        assert_eq!(legacy_hint("capture"), None);
        assert_eq!(legacy_hint("settings"), None);
    }

    #[test]
    fn the_table_constants_are_distinct() {
        let codes = [
            OK,
            GENERIC,
            USAGE,
            CANCELLED,
            CAPTURE_BACKEND,
            PERMISSION_DENIED,
            ACTION_EXPORT,
        ];
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[..index].contains(code), "duplicate code {code}");
        }
    }
}
