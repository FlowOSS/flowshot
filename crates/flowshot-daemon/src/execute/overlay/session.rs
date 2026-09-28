//! The overlay session leg: the child's blocking winit run,
//! its parameter bundle, and the shared frame/geometry helpers.

use std::path::PathBuf;
use std::sync::Arc;

use flowshot_capture::BackendKind;
use flowshot_capture_wayland::CapturedOutputs;
use flowshot_core::Config;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputLayout};
use flowshot_ui::launcher::RegionGeometry;
use flowshot_ui::{
    BackdropOptions, Completion, FramePixels, FrozenCapture, OverlayRuntime, SelectionConfig,
};

use super::wiring::{CoreSinks, SessionEvents, configure_core};
use crate::request::CaptureRequest;

/// Everything the child's overlay leg needs.
#[derive(Debug)]
pub struct OverlaySession {
    /// The frozen per-output frames + cursor sprite.
    pub frozen: FrozenCapture,
    /// The editor's sampling frame (stitched, scale 1.0 - see
    /// [`stitched_editor_frame`]).
    pub editor_frame: Option<FramePixels>,
    /// Loaded configuration.
    pub config: Config,
    /// Where `[capture].last_region` / `[editor].draw_color` persist.
    pub config_path: Option<PathBuf>,
    /// The invocation modifiers.
    pub request: CaptureRequest,
    /// Resolved cursor (the cursor ladder) for the launch preselect.
    pub cursor: Option<LogicalPoint>,
    /// `flowshot color`: the eyedropper is the whole session.
    pub color_mode: bool,
}

/// How one overlay session ended.
#[derive(Debug)]
pub enum SessionOutcome {
    /// A capture-completing gesture fired; the export is in hand.
    Completed(Completion),
    /// The standalone color pick completed (hex recorded; the parent owns
    /// the clipboard copy).
    ColorPicked(String),
    /// Esc/exit without any completion gesture.
    Cancelled,
    /// The window runtime failed.
    Failed(flowshot_ui::UiError),
}

/// The blocking winit leg (the session child's main thread): builds the
/// runtime, applies [`configure_core`], runs until teardown, and reports
/// what the sinks delivered.
#[must_use]
pub fn run_overlay_session(session: OverlaySession) -> SessionOutcome {
    let hide_cursor = session.request.hide_cursor || session.config.capture.hide_cursor;
    let options = BackdropOptions {
        dim: true,
        cursor_visible: !hide_cursor,
        selection: None,
    };
    let selection_config = SelectionConfig::from_editor(&session.config.editor);
    let runtime = match OverlayRuntime::with_capture_configured(
        session.frozen.clone(),
        options,
        selection_config,
    ) {
        Ok(runtime) => {
            runtime.with_window_customizer(flowshot_ui::pins::WindowCustomizer::new(|attributes| {
                use winit::platform::wayland::WindowAttributesExtWayland;
                attributes.with_name("flowshot", "FlowShot")
            }))
        }
        Err(error) => return SessionOutcome::Failed(error),
    };
    let events = Arc::new(SessionEvents::default());
    let mut runtime = runtime;
    let exit = runtime.handle().clone();
    configure_core(
        runtime.core_mut(),
        &session.config,
        &session.request,
        session.cursor,
        session.color_mode,
        session.editor_frame,
        &CoreSinks {
            config_path: session.config_path,
            events: Arc::clone(&events),
            exit: Some(exit),
        },
    );
    runtime.retheme_backdrop();
    match runtime.run() {
        Ok(()) => events.take_outcome(),
        Err(error) => SessionOutcome::Failed(error),
    }
}

/// The editor's sampling frame (pixelate / magnifier /
/// eyedropper read side). DECISION (recorded): the STITCHED scale-1
/// composite of the whole layout -
/// every output is sampleable (the per-output alternative leaves the
/// magnifier/pixelate blind on all but the first output); the trade is
/// logical-resolution sampling on scale>1 outputs (documented degradation,
/// exact at scale 1).
#[must_use]
pub fn stitched_editor_frame(
    frozen: &FrozenCapture,
    kind: BackendKind,
    layout: &OutputLayout,
) -> Option<FramePixels> {
    let union = layout.union_bounds()?;
    let captured = CapturedOutputs {
        outputs: frozen.outputs.clone(),
        frames: frozen.frames.clone(),
    };
    let stitched = captured.stitch(kind, union).ok()?;
    Some(FramePixels {
        rgba: stitched.buffer.data.to_vec(),
        width: stitched.buffer.width,
        height: stitched.buffer.height,
        scale: 1.0,
        origin: LogicalPoint::from_raw(union.x.0, union.y.0),
    })
}

/// The capture rect a typed geometry covers (the launcher dispatch +
/// direct-region captures): explicit offsets land as-is, offset-less
/// `WxH` centers at the resolved cursor (the geometry semantics); `None`
/// when offset-less without a cursor.
#[must_use]
pub fn region_rect_of(
    geometry: &RegionGeometry,
    cursor: Option<LogicalPoint>,
) -> Option<LogicalRect> {
    let (width, height) = (f64::from(geometry.width), f64::from(geometry.height));
    let (x, y) = if let (Some(x), Some(y)) = (geometry.x, geometry.y) {
        (f64::from(x), f64::from(y))
    } else {
        let at = cursor?;
        (at.x.0 - width / 2.0, at.y.0 - height / 2.0)
    };
    Some(LogicalRect::from_raw(x, y, width, height))
}
