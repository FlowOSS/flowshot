//! The pin action host (plan todo 30/38): a completed capture becomes a
//! floating pin window in a dedicated session CHILD (the winit
//! one-event-loop-per-process constraint, see [`super::session`]). The
//! PARENT registers the pin in the daemon registry before the spawn (the
//! "pins alive" persistence reason) and unregisters when the child exits;
//! the child bridges the copy/save menu items onto the actions crate (the
//! `pin_window` harness's `ActionBridge` was the reference).
//!
//! Documented trade-off: a pin's clipboard offer is served by the pin
//! child, so it dies when the pin closes (the daemon-owned offer contract
//! of todo 28 covers CAPTURE clipboard; a pin-copy outliving its pin
//! would need a bus-level clipboard handoff - recorded in issues.md).

use std::sync::Arc;
use std::time::SystemTime;

use flowshot_actions::Clipboard;
use flowshot_actions::pin::{PinRecord, copy_pin, save_pin};
use flowshot_core::Config;
use flowshot_core::config::SaveConfig;
use flowshot_ui::ExportedImage;
use flowshot_ui::pins::{
    PinActionSink, PinBehavior, PinId, PinImage, PinRuntime, PinSnapshot, PinSpec, WindowCustomizer,
};
use winit::platform::wayland::WindowAttributesExtWayland;

use super::post::PicturesDirDialog;
use super::session::{self, PinParams, SessionKind, SessionResult, SessionSpec};
use super::{ExecCtx, ExecuteError};

/// Hosts one pin: spawns the session child and (daemon mode) tracks it in
/// the registry. Returns when the pin window closes - the one-shot CLI
/// process must outlive its pins, and the daemon's execution thread is
/// dedicated anyway (the `pins_alive` reason holds the lifecycle).
///
/// # Errors
///
/// [`ExecuteError::Io`] for the handoff-PNG write, session-spawn failures
/// per [`session::spawn`], and [`ExecuteError::Child`] when the pin child
/// failed.
pub async fn spawn_pin(
    image: &ExportedImage,
    config: &Config,
    ctx: &ExecCtx,
) -> Result<(), ExecuteError> {
    let id = next_pin_id(ctx.state.as_ref());
    let image_path = session::temp_path(&format!("pin-{id}.png"));
    write_pin_png(&image_path, image)?;
    if let Some(state) = ctx.state.as_ref() {
        state.register_pin(PinRecord {
            id,
            width: image.width,
            height: image.height,
            opened_at: SystemTime::now(),
        });
    }
    let result = session::spawn(SessionSpec {
        kind: SessionKind::Pin,
        result_path: std::path::PathBuf::new(),
        image_path: Some(image_path.clone()),
        config_path: ctx.config_path.clone(),
        request: crate::request::CaptureRequest::default(),
        color_mode: false,
        forward_to_daemon: false,
        pin: Some(PinParams {
            id,
            min_size: config.pin.min_size,
        }),
    })
    .await;
    if let Some(state) = ctx.state.as_ref() {
        state.unregister_pin(id);
    }
    let _ = std::fs::remove_file(&image_path);
    match result? {
        SessionResult::Closed | SessionResult::Cancelled => Ok(()),
        SessionResult::Failed { error, exit_code } => Err(ExecuteError::Child { error, exit_code }),
        other => Err(ExecuteError::Task(format!(
            "the pin child reported an unexpected result: {other:?}"
        ))),
    }
}

fn write_pin_png(path: &std::path::Path, image: &ExportedImage) -> Result<(), ExecuteError> {
    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone()).ok_or(
        flowshot_actions::ExportError::ImageEncode(
            "pin image dimensions disagree with its pixels".to_owned(),
        ),
    )?;
    rgba.save(path)
        .map_err(|error| flowshot_actions::ExportError::ImageEncode(error.to_string()))?;
    Ok(())
}

fn next_pin_id(state: Option<&Arc<crate::state::DaemonState>>) -> u64 {
    let alive = state.map_or(0, |state| state.pin_ids().len());
    u64::try_from(alive).unwrap_or(0).saturating_add(1)
}

/// The child-side pin session (main thread): load the handoff PNG, run
/// the pin runtime until the window closes.
#[must_use]
pub fn pin_child(spec: &SessionSpec) -> SessionResult {
    let Some(pin) = spec.pin.as_ref() else {
        return failed("the pin spec carries no pin parameters".to_owned());
    };
    let Some(image_path) = spec.image_path.as_ref() else {
        return failed("the pin spec carries no image path".to_owned());
    };
    let image = match load_pin_image(image_path) {
        Ok(image) => image,
        Err(error) => return failed(format!("pin image load failed: {error}")),
    };
    let config = session::load_config(spec.config_path.as_deref());
    let bridge = Arc::new(ActionBridge {
        clipboard: Clipboard::wayland(),
        save_config: config.save.clone(),
    });
    let behavior = PinBehavior {
        min_size: f64::from(pin.min_size),
        ..PinBehavior::default()
    };
    let spec_window = PinSpec {
        id: PinId::new(pin.id),
        image,
    };
    let runtime = match PinRuntime::new(vec![spec_window], Some(bridge), behavior) {
        Ok(runtime) => runtime,
        Err(error) => return failed(format!("pin runtime startup failed: {error}")),
    };
    let runtime = runtime.with_window_customizer(WindowCustomizer::new(|attributes| {
        attributes.with_name("flowshot-pin", "FlowShot Pin")
    }));
    tracing::info!(pin = pin.id, "pin window session started");
    match runtime.run() {
        Ok(()) => {
            tracing::info!(pin = pin.id, "pin window session closed");
            SessionResult::Closed
        }
        Err(error) => failed(format!("pin runtime failed: {error}")),
    }
}

fn load_pin_image(path: &std::path::Path) -> Result<PinImage, String> {
    let loaded = image::open(path)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = loaded.dimensions();
    Ok(PinImage {
        width,
        height,
        rgba: loaded.into_raw(),
    })
}

fn failed(error: String) -> SessionResult {
    SessionResult::Failed {
        error,
        exit_code: 1,
    }
}

/// The binary-layer bridge: UI callback trait -> actions-crate modules
/// (the `pin_window` harness pattern). The registry bookkeeping lives in
/// the PARENT (the child has no daemon state).
#[derive(Debug)]
struct ActionBridge {
    clipboard: Clipboard,
    save_config: SaveConfig,
}

impl PinActionSink for ActionBridge {
    fn copy(&self, snapshot: PinSnapshot) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let image = flowshot_actions::PinImage {
            width: snapshot.width,
            height: snapshot.height,
            rgba: snapshot.rgba,
        };
        copy_pin(&image, &self.save_config, &self.clipboard)?;
        tracing::info!(pin = snapshot.id.raw(), "pin copied to clipboard");
        Ok(())
    }

    fn save(&self, snapshot: PinSnapshot) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let image = flowshot_actions::PinImage {
            width: snapshot.width,
            height: snapshot.height,
            rgba: snapshot.rgba,
        };
        let path = save_pin(
            &image,
            &self.save_config,
            &PicturesDirDialog,
            &flowshot_actions::export::NullNotifySink,
        )?;
        tracing::info!(pin = snapshot.id.raw(), path = %path.display(), "pin saved");
        Ok(())
    }

    fn pin_closed(&self, id: PinId) {
        tracing::info!(pin = id.raw(), "pin window closed");
    }
}
