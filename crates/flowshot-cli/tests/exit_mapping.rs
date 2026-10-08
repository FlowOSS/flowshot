//! The permission-denied / failure-class exit-code
//! mapping - the executor's typed errors land on the
//! table exactly, including the protocol permission-refusal chain
//! (`IccError::PermissionDenied` inside `CaptureError::Backend` -> 5).

#![allow(clippy::unwrap_used)]

use flowshot_cli::exit::{
    ACTION_EXPORT, CANCELLED, CAPTURE_BACKEND, OK, PERMISSION_DENIED, USAGE, exec_exit_u8,
};
use flowshot_daemon::execute::{ExecOutcome, ExecuteError};

fn code(result: &Result<ExecOutcome, ExecuteError>) -> u8 {
    exec_exit_u8(result)
}

#[test]
fn permission_denied_chain_maps_to_five() {
    let error = ExecuteError::Capture(flowshot_capture::CaptureError::Backend {
        backend: flowshot_capture::BackendKind::ExtImageCopyCapture,
        source: Box::new(flowshot_capture_wayland::IccError::PermissionDenied),
    });
    assert_eq!(code(&Err(error)), PERMISSION_DENIED);
}

#[test]
fn capture_backend_failures_map_to_four() {
    let error = ExecuteError::Capture(flowshot_capture::CaptureError::NoBackendAvailable {
        missing: vec![flowshot_capture::BackendKind::ExtImageCopyCapture],
    });
    assert_eq!(code(&Err(error)), CAPTURE_BACKEND);
}

#[test]
fn export_failures_map_to_six() {
    let error = ExecuteError::Export(flowshot_actions::ExportError::Cancelled);
    assert_eq!(code(&Err(error)), ACTION_EXPORT);
}

#[test]
fn usage_failures_map_to_two() {
    let error = ExecuteError::Usage("bad argv".to_owned());
    assert_eq!(code(&Err(error)), USAGE);
}

#[test]
fn child_failures_propagate_their_mapped_code() {
    let error = ExecuteError::Child {
        error: "overlay startup failed".to_owned(),
        exit_code: CAPTURE_BACKEND,
    };
    assert_eq!(code(&Err(error)), CAPTURE_BACKEND);
}

#[test]
fn cancellation_maps_to_three_and_success_to_zero() {
    assert_eq!(code(&Ok(ExecOutcome::Cancelled)), CANCELLED);
    assert_eq!(
        code(&Ok(ExecOutcome::Done(
            flowshot_actions::clipboard::PostCaptureReport::default()
        ))),
        OK
    );
}
