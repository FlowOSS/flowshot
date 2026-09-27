//! The launcher child's Capture dispatch legs (todo 37/38): the
//! daemon-resident bus forward (the production mapping) and the one-shot
//! in-child direct capture (a second event loop is impossible in the
//! child; the todo-37 harness-proven semantics).

use std::collections::HashMap;
use std::time::Instant;

use flowshot_ui::launcher::LauncherRequest;

use super::super::overlay::region_rect_of;
use super::super::session::{self, SessionResult, SessionSpec};
use super::super::{ExecuteError, direct};

/// The daemon-resident dispatch: the typed bus members (todo-37 mapping).
pub(super) fn forward_to_daemon(request: &LauncherRequest) -> SessionResult {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return SessionResult::Failed {
                error: format!("launcher bus runtime failed: {error}"),
                exit_code: 1,
            };
        }
    };
    let outcome = runtime.block_on(bus_dispatch(request));
    match outcome {
        Ok(()) => SessionResult::Dispatched,
        Err(error) => SessionResult::Failed {
            error: error.to_string(),
            exit_code: session::exit::code_for(&error),
        },
    }
}

async fn bus_dispatch(request: &LauncherRequest) -> Result<(), ExecuteError> {
    use zbus::zvariant::{Str, Value};
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| ExecuteError::Task(format!("session bus connect failed: {error}")))?;
    let result = match request {
        LauncherRequest::Region { geometry, delay_ms } => {
            let mut options: HashMap<String, Value<'_>> = HashMap::new();
            options.insert(
                "region".to_owned(),
                Value::Str(Str::from(geometry.to_token())),
            );
            if *delay_ms > 0 {
                options.insert("delay_ms".to_owned(), Value::U32(*delay_ms));
            }
            connection
                .call_method(
                    Some(crate::SERVICE),
                    crate::OBJECT_PATH,
                    Some(crate::IFACE),
                    "Capture",
                    &options,
                )
                .await
        }
        LauncherRequest::Screen { screen, delay_ms } => {
            let mut argv = vec![
                "capture".to_owned(),
                "screen".to_owned(),
                screen.to_string(),
            ];
            if *delay_ms > 0 {
                argv.push("-d".to_owned());
                argv.push(delay_ms.to_string());
            }
            connection
                .call_method(
                    Some(crate::SERVICE),
                    crate::OBJECT_PATH,
                    Some(crate::IFACE),
                    "Invoke",
                    &argv,
                )
                .await
        }
    };
    result.map_err(|error| ExecuteError::Task(format!("launcher bus dispatch failed: {error}")))?;
    connection.close().await.ok();
    Ok(())
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
