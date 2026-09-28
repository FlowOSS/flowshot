//! The production core wiring: `configure_core` is the SINGLE
//! function both the live session child and the headless execution mode
//! drive, so the offscreen QA path exercises the exact production
//! configuration (tools, editor frame + config, chrome projection,
//! persistence sinks, launch preselect, completion + color sinks).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use flowshot_core::Config;
use flowshot_core::geometry::{LogicalPoint, LogicalSize};
use flowshot_ui::launch::{LaunchRequest, Preselect};
use flowshot_ui::launcher::RegionGeometry;
use flowshot_ui::{Completion, CompletionSink, FramePixels, OverlayCore, ToolKind};

use crate::request::CaptureRequest;

/// The sink-delivered session events (completion / color pick).
#[derive(Debug, Default)]
pub struct SessionEvents {
    completion: Mutex<Option<Completion>>,
    color: Mutex<Option<String>>,
}

impl SessionEvents {
    /// Drains the delivered outcome (completion first, then color pick,
    /// else cancelled).
    pub(crate) fn take_outcome(&self) -> super::session::SessionOutcome {
        if let Some(completion) = self
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return super::session::SessionOutcome::Completed(completion);
        }
        if let Some(hex) = self
            .color
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return super::session::SessionOutcome::ColorPicked(hex);
        }
        super::session::SessionOutcome::Cancelled
    }
}

/// The sink bundle [`configure_core`] installs.
#[derive(Clone)]
pub struct CoreSinks {
    /// The TOML the region-memory and draw-color sinks write.
    pub config_path: Option<PathBuf>,
    /// Where completion/color events land.
    pub events: Arc<SessionEvents>,
    /// Loop exit handle (the color pick tears down after recording);
    /// `None` in the headless mode (no loop to exit).
    pub exit: Option<flowshot_ui::OverlayHandle>,
}

impl std::fmt::Debug for CoreSinks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoreSinks")
            .field("config_path", &self.config_path)
            .field("exit", &self.exit.is_some())
            .finish_non_exhaustive()
    }
}

/// THE production wiring (the `frozen_backdrop` harness's
/// `apply_launch_args` was the reference).
pub fn configure_core(
    core: &mut OverlayCore,
    config: &Config,
    request: &CaptureRequest,
    cursor: Option<LogicalPoint>,
    color_mode: bool,
    editor_frame: Option<FramePixels>,
    sinks: &CoreSinks,
) {
    flowshot_ui::register_shape_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_text_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_pixelate_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_counter_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_selection_tools(core.editor_mut().registry_mut());
    core.install_frame(editor_frame);
    core.editor_mut()
        .configure(flowshot_ui::EditorTools::from_config(config));
    core.configure_chrome(&config.ui);
    if color_mode {
        core.editor_mut().activate_tool(ToolKind::Eyedropper);
    }
    if let Some(path) = sinks.config_path.clone() {
        let writer = path.clone();
        core.chrome_mut()
            .set_draw_color_sink(Some(Box::new(move |hex: &str| {
                persist(&writer, |config| {
                    hex.clone_into(&mut config.editor.draw_color);
                });
            })));
        core.set_region_sink(Some(Box::new(move |region| {
            persist(&path, |config| config.capture.last_region = Some(region));
        })));
    }
    let events = Arc::clone(&sinks.events);
    core.set_completion_sink(Some(Box::new(move |completion: Completion| {
        tracing::info!(
            target: "flowshot_perf",
            kind = ?completion.kind,
            width = completion.image.width,
            height = completion.image.height,
            "perf.completion"
        );
        *events
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(completion);
    }) as CompletionSink));
    if color_mode {
        let events = Arc::clone(&sinks.events);
        let exit = sinks.exit.clone();
        core.set_color_pick_sink(Some(Box::new(move |color| {
            let hex = format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b);
            tracing::info!(hex = %hex, "color picked");
            *events
                .color
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hex);
            if let Some(handle) = exit.as_ref() {
                let _ = handle.request_exit();
            }
        })));
    }
    core.launch(build_launch_request(config, request, cursor));
}

/// Maps the wire request + config onto the typed launch request (the
/// launch.rs module-header MAPPING TABLE).
#[must_use]
pub fn build_launch_request(
    config: &Config,
    request: &CaptureRequest,
    cursor: Option<LogicalPoint>,
) -> LaunchRequest {
    LaunchRequest {
        preselect: preselect_for(config, request),
        cursor,
        instant: request.instant || request.no_edit,
        save_last_region: config.capture.save_last_region,
    }
}

fn preselect_for(config: &Config, request: &CaptureRequest) -> Preselect {
    if request.last_region {
        return Preselect::LastRegion(config.capture.last_region);
    }
    let Some(token) = request.region.as_deref() else {
        return Preselect::None;
    };
    if token == "at-cursor" {
        return Preselect::OutputAtCursor;
    }
    match RegionGeometry::parse(token) {
        Ok(geometry) => Preselect::Region {
            size: LogicalSize::from_raw(f64::from(geometry.width), f64::from(geometry.height)),
            origin: match (geometry.x, geometry.y) {
                (Some(x), Some(y)) => Some(LogicalPoint::from_raw(f64::from(x), f64::from(y))),
                _ => None,
            },
        },
        Err(issue) => {
            tracing::warn!(
                token,
                ?issue,
                "region token invalid; launching without preselect"
            );
            Preselect::None
        }
    }
}

fn persist(path: &std::path::Path, edit: impl FnOnce(&mut Config)) {
    let mut config = Config::load(path).unwrap_or_default();
    edit(&mut config);
    if let Err(error) = config.save(path) {
        tracing::warn!(%error, "config persistence failed");
    }
}
