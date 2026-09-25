//! Conversion from winit monitor handles into the core [`OutputLayout`].
//!
//! Thin startup glue (no tests per draft D4 - correctness of the algebra
//! lives in `flowshot_core::geometry` and the router).
//!
//! # Semantics of the winit monitor report
//!
//! `MonitorHandle::position()` is physical pixels (the portable layer scales
//! the compositor's logical position by the scale factor), `size()` is the
//! output's native mode size in physical pixels, and the output transform is
//! not exposed. The conversion therefore divides the position back into
//! global logical space via the core `ToLogical` conversion and assumes
//! [`Transform::Normal`]. For rotated outputs the logical *size* derived from
//! the mode is wrong until the window's first `Resized` event refines it via
//! [`InputRouter::update_surface_size`](crate::InputRouter::update_surface_size);
//! todo 15 supersedes this layout entirely with the capture-provided one
//! (true transforms included).

use flowshot_core::geometry::{
    LogicalRect, OutputInfo, OutputLayout, PhysicalPoint, PhysicalSize, ToLogical, ToPhysical,
    Transform,
};
use winit::monitor::MonitorHandle;

use crate::error::UiError;

/// Sentinel binding for a window whose monitor matches no captured output;
/// out-of-range indices route to `None` (the router's unbound-slot contract)
/// and render the letterbox placeholder.
const UNBOUND: usize = usize::MAX;

/// Binds winit monitors to a capture-provided layout (plan todo 15: the
/// capture layout supersedes the monitor-derived one - true transforms and
/// scales included).
///
/// Matching is by connector name first (winit reports the Wayland output
/// name), then by physical origin. Returns the layout clone plus one output
/// index per monitor ([`UNBOUND`] when no output matches).
#[must_use]
pub(crate) fn bindings_for_layout(
    layout: &OutputLayout,
    monitors: &[MonitorHandle],
) -> (OutputLayout, Vec<usize>) {
    let bindings = monitors
        .iter()
        .map(|monitor| match_output(layout, monitor))
        .collect();
    (layout.clone(), bindings)
}

fn match_output(layout: &OutputLayout, monitor: &MonitorHandle) -> usize {
    let name = monitor.name();
    if let Some(name) = name.as_deref()
        && let Some(index) = layout
            .outputs
            .iter()
            .position(|output| output.connector == name)
    {
        return index;
    }
    let position = monitor.position();
    layout
        .outputs
        .iter()
        .position(|output| {
            let origin = output.logical_rect.origin().to_physical(output.scale);
            origin.x.0 == position.x && origin.y.0 == position.y
        })
        .unwrap_or(UNBOUND)
}

/// Builds the desktop layout from the compositor's monitor report.
///
/// Output order matches `monitors` order, so window slot `i` binds to output
/// `i`.
///
/// # Errors
///
/// Returns [`UiError::NoMonitors`] for an empty report,
/// [`UiError::MonitorSizeOverflow`] when a size exceeds `i32` bounds, and
/// [`UiError::MonitorGeometry`] when core validation rejects a monitor.
pub(crate) fn layout_from_monitors(monitors: &[MonitorHandle]) -> Result<OutputLayout, UiError> {
    if monitors.is_empty() {
        return Err(UiError::NoMonitors);
    }
    let outputs = monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| output_from_monitor(monitor, index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OutputLayout::new(outputs))
}

/// The display name winit reports for `monitor`, with a stable fallback.
pub(crate) fn monitor_name(monitor: &MonitorHandle, index: usize) -> String {
    monitor.name().unwrap_or_else(|| format!("monitor-{index}"))
}

fn output_from_monitor(monitor: &MonitorHandle, index: usize) -> Result<OutputInfo, UiError> {
    let name = monitor_name(monitor, index);
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let position = monitor.position();
    let to_px = |value: u32| {
        i32::try_from(value).map_err(|_| UiError::MonitorSizeOverflow {
            name: name.clone(),
            width: size.width,
            height: size.height,
        })
    };
    let physical = PhysicalSize::from_raw(to_px(size.width)?, to_px(size.height)?);
    let origin = PhysicalPoint::from_raw(position.x, position.y).to_logical(scale);
    let logical_size = physical.to_logical(scale);
    let logical_rect = LogicalRect::from_parts(origin, logical_size);
    let built = OutputInfo::new(
        name.clone(),
        name.clone(),
        logical_rect,
        physical,
        scale,
        Transform::Normal,
    );
    built.map_err(|source| UiError::MonitorGeometry { name, source })
}
