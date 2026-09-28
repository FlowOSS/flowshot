//! The launcher child's Capture dispatch legs (todo 37/38): the
//! daemon-resident argv handoff and the one-shot in-child direct capture.
//!
//! F3 fix (2026-09-28, live-QA found): the daemon-resident leg used to
//! call the bus FROM the child while the launcher session was alive - the
//! parent's `SessionGuard` (single window session) rejected the dispatched
//! Capture every time ("a `FlowShot` window session is already active"), so
//! the dialog's Capture button could never run a capture in daemon mode.
//! The child now returns the dispatch as DATA (`SessionResult::Dispatched
//! { argv }`); the parent executes it through the lossless `Invoke`
//! channel after `session::spawn` returns and the guard releases.

use std::time::Instant;

use flowshot_ui::launcher::LauncherRequest;

use super::super::overlay::region_rect_of;
use super::super::session::{self, SessionResult, SessionSpec};
use super::super::{ExecuteError, direct};

/// The daemon-resident dispatch: translate the request into the lossless
/// `Invoke` argv (the todo-37 mapping, same vocabulary the bus forward
/// used: `Region` -> `capture --region TOKEN [-d MS]` = interactive
/// overlay preselect; `Screen` -> `capture screen N [-d MS]`).
pub(super) fn dispatch_argv(request: &LauncherRequest) -> SessionResult {
    let mut argv = vec!["capture".to_owned()];
    match request {
        LauncherRequest::Region { geometry, delay_ms } => {
            argv.push("--region".to_owned());
            argv.push(geometry.to_token());
            push_delay(&mut argv, *delay_ms);
        }
        LauncherRequest::Screen { screen, delay_ms } => {
            argv.push("screen".to_owned());
            argv.push(screen.to_string());
            push_delay(&mut argv, *delay_ms);
        }
    }
    SessionResult::Dispatched { argv }
}

fn push_delay(argv: &mut Vec<String>, delay_ms: u32) {
    if delay_ms > 0 {
        argv.push("-d".to_owned());
        argv.push(delay_ms.to_string());
    }
}

/// The one-shot dispatch: capture the typed geometry directly in the
/// child (no second event loop exists for an overlay; the todo-37 harness
/// semantics) and write the export PNG for the parent's post-capture.
pub(super) fn capture_in_child(request: &LauncherRequest, spec: &SessionSpec) -> SessionResult {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return SessionResult::Failed {
                error: format!("launcher capture runtime failed: {error}"),
                exit_code: 1,
            };
        }
    };
    let started = Instant::now();
    let outcome = runtime.block_on(async {
        let (target, delay_ms) = match request {
            LauncherRequest::Region { geometry, delay_ms } => {
                let cursor = crate::execute::backend::resolve_cursor().await;
                let Some(rect) = region_rect_of(geometry, cursor) else {
                    return Err(ExecuteError::Usage(
                        "offset-less launcher geometry needs a resolved cursor position".to_owned(),
                    ));
                };
                (direct::Target::Region(rect), *delay_ms)
            }
            LauncherRequest::Screen { screen, delay_ms } => (
                direct::Target::Screen(direct::ScreenTarget::Index(*screen)),
                *delay_ms,
            ),
        };
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(u64::from(delay_ms))).await;
        }
        let config = session::load_config(spec.config_path.as_deref());
        let hide_cursor = config.capture.hide_cursor;
        direct::capture_and_composite(&target, hide_cursor, started).await
    });
    match outcome {
        Ok(completion) => {
            let Some(image_path) = spec.image_path.as_ref() else {
                return SessionResult::Failed {
                    error: "the launcher spec carries no image path".to_owned(),
                    exit_code: 1,
                };
            };
            let Some(rgba) = image::RgbaImage::from_raw(
                completion.image.width,
                completion.image.height,
                completion.image.rgba.clone(),
            ) else {
                return SessionResult::Failed {
                    error: "launcher export dimensions disagree with its pixels".to_owned(),
                    exit_code: 1,
                };
            };
            if let Err(error) = rgba.save(image_path) {
                return SessionResult::Failed {
                    error: format!("launcher export PNG write failed: {error}"),
                    exit_code: 6,
                };
            }
            SessionResult::Completed {
                kind: session::kind_token(completion.kind),
                selection: completion.selection,
            }
        }
        Err(error) => {
            let exit_code = session::exit::code_for(&error);
            SessionResult::Failed {
                error: error.to_string(),
                exit_code,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use flowshot_ui::launcher::RegionGeometry;

    use super::*;
    use crate::execute::direct::{ScreenTarget, Target};
    use crate::execute::invoke::{self, InvokeCall};

    fn argv_of(result: SessionResult) -> Vec<String> {
        match result {
            SessionResult::Dispatched { argv } => argv,
            other => panic!("expected a dispatch, got {other:?}"),
        }
    }

    #[test]
    fn region_dispatch_round_trips_through_the_invoke_parser() {
        // Given: a manual-geometry launcher request with a delay
        let geometry = RegionGeometry::parse("100x100+50+50").expect("valid geometry");
        let request = LauncherRequest::Region {
            geometry,
            delay_ms: 2000,
        };
        // When: translated to the parent-executed argv
        let argv = argv_of(dispatch_argv(&request));
        // Then: the Invoke parser reads it back as the interactive preselect
        match invoke::parse(&argv).expect("parses") {
            InvokeCall::Interactive(parsed) => {
                assert_eq!(parsed.region.as_deref(), Some("100x100+50+50"));
                assert_eq!(parsed.delay_ms, 2000);
            }
            other => panic!("expected Interactive, got {other:?}"),
        }
    }

    #[test]
    fn screen_dispatch_round_trips_through_the_invoke_parser() {
        // Given: a monitor-index launcher request without a delay
        let request = LauncherRequest::Screen {
            screen: 1,
            delay_ms: 0,
        };
        // When: translated to the parent-executed argv
        let argv = argv_of(dispatch_argv(&request));
        // Then: the Invoke parser reads it back as the direct screen capture
        match invoke::parse(&argv).expect("parses") {
            InvokeCall::Direct(Target::Screen(ScreenTarget::Index(1)), parsed) => {
                assert_eq!(parsed.delay_ms, 0);
            }
            other => panic!("expected Direct(Screen(1)), got {other:?}"),
        }
    }
}
