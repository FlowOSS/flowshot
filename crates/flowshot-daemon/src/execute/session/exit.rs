//! The shared exit-code mapping (the todo-35 table; the CLI's `exit.rs`
//! constants are the user-facing documentation of these numbers). Extracted
//! from [`super`] at the 250-LOC ceiling.

use crate::execute::ExecuteError;

/// Maps one executor error onto the shared exit-code table.
#[must_use]
pub fn code_for(error: &ExecuteError) -> u8 {
    match error {
        ExecuteError::Usage(_) => 2,
        ExecuteError::Capture(_) if is_permission_denied(error) => 5,
        ExecuteError::Capture(_) | ExecuteError::Probe(_) | ExecuteError::Connect(_) => 4,
        ExecuteError::Export(_) | ExecuteError::Clipboard(_) => 6,
        ExecuteError::Child { exit_code, .. } => *exit_code,
        ExecuteError::Ui(_)
        | ExecuteError::Io(_)
        | ExecuteError::Task(_)
        | ExecuteError::Daemon(_) => 1,
    }
}

/// The code for a child that died without a result (infrastructure).
#[must_use]
pub const fn exit_code_for_child_failure() -> u8 {
    1
}

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
