//! The headless execution mode (feature `test-drive`).
//!
//! The minimal honest stand-in for the visible overlay session: the SAME
//! production wiring ([`super::overlay::configure_core`]), the SAME input
//! funnel (`OverlayCore::inject_event` - the test seam), and the
//! SAME export implementation ([`flowshot_ui::render_export`] ->
//! `composite_selection`) the live shell runs - only the winit/Wayland
//! window leg is replaced by offscreen GPU renders. A virtual seat would
//! need a nested compositor, which is forbidden; the window leg itself
//! carries per-module live evidence and the deferred GUI-QA
//! batch.
//!
//! The shell's completion trigger is emulated exactly: when the funnel
//! reports a capture-completing action, the driver renders the export and
//! delivers it through the INSTALLED completion sink
//! ([`OverlayCore::deliver_completion`]), so the sink wiring under test is
//! the production one.

use flowshot_core::Config;
use flowshot_core::geometry::{LogicalPoint, OutputLayout};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::gpu::GpuContext;
use flowshot_ui::{
    Action, Backdrop, CompletionKind, FramePixels, FrozenCapture, InputRouter, OverlayCore,
    SyntheticInput, render_export,
};

use super::overlay::{CoreSinks, SessionEvents, SessionOutcome, configure_core};
use super::{ExecuteError, elapsed};
use crate::request::CaptureRequest;
use std::sync::Arc;
use std::time::Instant;

/// Everything one headless session needs.
#[derive(Debug)]
pub struct HeadlessSession {
    /// The frozen frames the backdrop plans from.
    pub frozen: FrozenCapture,
    /// The editor's sampling frame (see `overlay::stitched_editor_frame`).
    pub editor_frame: Option<FramePixels>,
    /// Loaded configuration.
    pub config: Config,
    /// Where the persistence sinks write (`None` disables them).
    pub config_path: Option<std::path::PathBuf>,
    /// The invocation modifiers.
    pub request: CaptureRequest,
    /// The resolved cursor for the launch preselect.
    pub cursor: Option<LogicalPoint>,
    /// `flowshot color` mode.
    pub color_mode: bool,
    /// Whether the export keeps the captured cursor sprite.
    pub cursor_visible: bool,
    /// The scripted input, in order (the production funnel consumes it).
    pub inputs: Vec<SyntheticInput>,
    /// QA-only tool rebinds applied after [`configure_core`] (the
    /// `frozen_backdrop` harness precedent: counter/blur/move ship
    /// unbound - panel-driven - so scripts rebind them for coverage).
    pub rebinds: Vec<(flowshot_ui::ToolKind, winit::keyboard::KeyCode)>,
}

/// Drives one headless session and reports the sink-delivered outcome.
///
/// # Errors
///
/// [`ExecuteError::Ui`] for GPU/render/readback failures.
pub fn run_headless(session: HeadlessSession) -> Result<SessionOutcome, ExecuteError> {
    let started = Instant::now();
    let layout = OutputLayout::new(session.frozen.outputs.clone());
    let bindings: Vec<usize> = (0..layout.outputs.len()).collect();
    let mut core = OverlayCore::new(InputRouter::new(layout, bindings));
    let events = Arc::new(SessionEvents::default());
    configure_core(
        &mut core,
        &session.config,
        &session.request,
        session.cursor,
        session.color_mode,
        session.editor_frame,
        &CoreSinks {
            config_path: session.config_path,
            events: Arc::clone(&events),
            exit: None,
        },
    );
    for (kind, key) in &session.rebinds {
        core.editor_mut().shortcuts_mut().rebind(*kind, Some(*key));
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: flowshot_ui::gpu::OVERLAY_BACKENDS,
        ..wgpu::InstanceDescriptor::default()
    });
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        "perf.gpu_instance"
    );
    let gpu = GpuContext::new_headless(&instance)?;
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        "perf.gpu_ready"
    );
    let mut backdrop = Backdrop::plan(session.frozen, &DesignTokens::default());
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        "perf.plan_ready"
    );
    for input in session.inputs {
        let report = core.inject_event(input);
        tracing::debug!(actions = ?report.actions, "headless input routed");
        let Some(kind) = completion_kind(&report.actions) else {
            continue;
        };
        let Some(selection) = core.selection().rect() else {
            tracing::warn!("completion action without a selection; ignored");
            continue;
        };
        if let Some(image) = render_export(
            &gpu,
            &mut backdrop,
            core.editor(),
            selection,
            session.cursor_visible,
        )? {
            tracing::info!(
                target: "flowshot_perf",
                elapsed_us = elapsed(started),
                width = image.width,
                height = image.height,
                "perf.frame_ready"
            );
            core.deliver_completion(flowshot_ui::Completion {
                kind,
                selection,
                image,
            });
        } else {
            tracing::error!("headless export composited no image");
        }
        break;
    }
    Ok(events.take_outcome())
}

fn completion_kind(actions: &[Action]) -> Option<CompletionKind> {
    actions.iter().find_map(|action| match action {
        Action::Accept => Some(CompletionKind::Accept),
        Action::Copy => Some(CompletionKind::Copy),
        Action::Save => Some(CompletionKind::Save),
        Action::Pin => Some(CompletionKind::Pin),
        Action::Upload => Some(CompletionKind::Upload),
        Action::OpenWith => Some(CompletionKind::OpenWith),
        Action::Redraw(_) | Action::Exit | Action::ColorWheel | Action::ColorPicked => None,
    })
}
