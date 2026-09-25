//! Central input router: window-local physical pixels to global logical space.
//!
//! This module is the cross-monitor spanning enabler (plan todo 13). During a
//! drag, the compositor's implicit pointer grab keeps delivering motion events
//! to the window where the drag started, even while the cursor is logically
//! over another monitor: positions arrive surface-local and *beyond the
//! surface bounds*. The router maps them linearly into the global logical
//! space of [`OutputLayout`] - never clamping to the source window - so a
//! selection started on monitor A continues seamlessly over monitor B.
//! Clamping to the layout is a separate, explicit operation
//! ([`InputRouter::clamp_point`]).
//!
//! The router is pure data - no windowing or GPU handles - so the mapping
//! math is unit-testable headlessly.
//!
//! # Coordinate spaces
//!
//! - *Surface-local physical*: fractional pixels relative to the top-left of
//!   one window's surface, as delivered by winit `CursorMoved`.
//! - *Global logical*: the shared desktop space of [`OutputLayout`], in which
//!   selections and crops are expressed (physical-first rule, todo 3).
//!
//! # Example
//!
//! ```
//! # fn main() -> Result<(), flowshot_core::geometry::GeometryError> {
//! use flowshot_core::geometry::{
//!     LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
//! };
//! use flowshot_ui::{InputRouter, WindowSlot};
//!
//! // Single 1920x1080 output at scale 1 anchored at the origin.
//! let output = OutputInfo::new(
//!     "DP-1",
//!     "DP-1",
//!     LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
//!     PhysicalSize::from_raw(1920, 1080),
//!     1.0,
//!     Transform::Normal,
//! )?;
//! let router = InputRouter::new(OutputLayout::new(vec![output]), vec![0]);
//!
//! let global = router.to_global(WindowSlot::new(0), 100.0, 200.0);
//! assert_eq!(global.map(|point| (point.x.0, point.y.0)), Some((100.0, 200.0)));
//! # Ok(())
//! # }
//! ```

use flowshot_core::geometry::{
    GeometryError, LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, ToLogical,
};

/// Identifies one overlay window (one per monitor), assigned densely at spawn.
///
/// Slots are the router's window keys: the winit shell maps opaque `WindowId`s
/// to slots, keeping the routing math free of platform handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowSlot(usize);

impl WindowSlot {
    /// Creates a slot from a raw index.
    ///
    /// Indices are assigned in monitor order at window spawn; a slot that was
    /// never registered simply routes to `None` (no panic).
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The raw index of this slot.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Maps per-window surface-local input into the global logical space of an
/// [`OutputLayout`].
#[derive(Debug, Clone, PartialEq)]
pub struct InputRouter {
    layout: OutputLayout,
    /// Output index per window slot; `None` marks an unbound slot.
    bindings: Vec<Option<usize>>,
}

impl InputRouter {
    /// Creates a router for `layout`.
    ///
    /// `bindings[i]` is the index into `layout.outputs` of the output that
    /// [`WindowSlot::new(i)`] fullscreen-covers. Out-of-range indices behave
    /// like unbound slots.
    #[must_use]
    pub fn new(layout: OutputLayout, bindings: Vec<usize>) -> Self {
        Self {
            layout,
            bindings: bindings.into_iter().map(Some).collect(),
        }
    }

    /// The layout this router maps into.
    #[must_use]
    pub const fn layout(&self) -> &OutputLayout {
        &self.layout
    }

    /// The number of registered window slots.
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.bindings.len()
    }

    /// The output bound to `slot`, when known.
    #[must_use]
    pub fn output_for(&self, slot: WindowSlot) -> Option<&OutputInfo> {
        let index = *self.bindings.get(slot.index())?;
        self.layout.outputs.get(index?)
    }

    /// Replaces the layout and slot bindings wholesale (window spawn).
    pub fn install(&mut self, layout: OutputLayout, bindings: Vec<usize>) {
        *self = Self::new(layout, bindings);
    }

    /// Replaces the layout, re-binding every slot to the output with the same
    /// connector name.
    ///
    /// Slots whose connector disappeared become unbound (route to `None`)
    /// until re-bound; used when the compositor reports monitor changes.
    pub fn replace_layout(&mut self, layout: OutputLayout) {
        let rebound = self
            .bindings
            .iter()
            .map(|binding| {
                let old = binding.and_then(|index| self.layout.outputs.get(index))?;
                layout
                    .outputs
                    .iter()
                    .position(|output| output.connector == old.connector)
            })
            .collect();
        self.bindings = rebound;
        self.layout = layout;
    }

    /// Maps a surface-local physical position (fractional px) to global
    /// logical space.
    ///
    /// The mapping is linear and deliberately *unclamped*: positions outside
    /// the window (implicit-grab drags) extend across the layout. Returns
    /// `None` for unknown slots and non-finite input.
    #[must_use]
    pub fn to_global(&self, slot: WindowSlot, local_x: f64, local_y: f64) -> Option<LogicalPoint> {
        if !local_x.is_finite() || !local_y.is_finite() {
            return None;
        }
        let output = self.output_for(slot)?;
        let scale = effective_scale(output.scale);
        Some(LogicalPoint::from_raw(
            output.logical_rect.x.0 + local_x / scale,
            output.logical_rect.y.0 + local_y / scale,
        ))
    }

    /// [`Self::to_global`] clamped to the layout's union bounds.
    #[must_use]
    pub fn to_global_clamped(
        &self,
        slot: WindowSlot,
        local_x: f64,
        local_y: f64,
    ) -> Option<LogicalPoint> {
        self.to_global(slot, local_x, local_y)
            .map(|point| self.clamp_point(point))
    }

    /// The inverse of [`Self::to_global`]: a global logical point as a
    /// surface-local physical position (fractional px) for `slot`.
    ///
    /// The result may lie outside the surface - that is expected when the
    /// point is over a neighboring monitor. Returns `None` for unknown slots
    /// and non-finite input.
    #[must_use]
    pub fn to_local(&self, slot: WindowSlot, global: LogicalPoint) -> Option<(f64, f64)> {
        if !global.x.0.is_finite() || !global.y.0.is_finite() {
            return None;
        }
        let output = self.output_for(slot)?;
        let scale = effective_scale(output.scale);
        Some((
            (global.x.0 - output.logical_rect.x.0) * scale,
            (global.y.0 - output.logical_rect.y.0) * scale,
        ))
    }

    /// Clamps a global logical point to the layout's union bounds (closed
    /// interval).
    ///
    /// Non-finite components collapse to the bounds minimum; an empty layout
    /// returns the point unchanged. Never panics.
    #[must_use]
    pub fn clamp_point(&self, point: LogicalPoint) -> LogicalPoint {
        let Some(bounds) = self.layout.union_bounds() else {
            return point;
        };
        LogicalPoint::from_raw(
            clamp_axis(point.x.0, bounds.x.0, bounds.right().0),
            clamp_axis(point.y.0, bounds.y.0, bounds.bottom().0),
        )
    }

    /// Refines the bound output's geometry from the window's actual surface
    /// extent (post-transform physical px, as reported by winit `Resized`).
    ///
    /// winit does not expose the output transform, so spawned outputs start
    /// with the monitor's pre-transform mode size and [`Transform::Normal`];
    /// the first `Resized` corrects size and derived logical rect for rotated
    /// outputs. Todo 15 supersedes this layout with the capture-provided one
    /// (true transforms included). Unbound slots are a silent no-op.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when the refined output fails core
    /// validation (e.g. a non-finite scale already stored in the layout).
    pub fn update_surface_size(
        &mut self,
        slot: WindowSlot,
        buffer: PhysicalSize,
    ) -> Result<(), GeometryError> {
        let Some(output) = self.output_mut(slot) else {
            return Ok(());
        };
        let logical_size = buffer.to_logical(output.scale);
        let rect = LogicalRect::from_parts(output.logical_rect.origin(), logical_size);
        let updated = OutputInfo::new(
            output.connector.clone(),
            output.name.clone(),
            rect,
            buffer,
            output.scale,
            output.transform,
        )?;
        *output = updated;
        Ok(())
    }

    /// Updates the bound output's scale factor, recomputing its logical size
    /// from the stored buffer size. Unbound slots are a silent no-op.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::InvalidScale`] when `scale` is not finite and
    /// positive.
    pub fn update_scale(&mut self, slot: WindowSlot, scale: f64) -> Result<(), GeometryError> {
        let Some(output) = self.output_mut(slot) else {
            return Ok(());
        };
        let logical_size = output.buffer_size().to_logical(scale);
        let rect = LogicalRect::from_parts(output.logical_rect.origin(), logical_size);
        let updated = OutputInfo::new(
            output.connector.clone(),
            output.name.clone(),
            rect,
            output.physical_size,
            scale,
            output.transform,
        )?;
        *output = updated;
        Ok(())
    }

    fn output_mut(&mut self, slot: WindowSlot) -> Option<&mut OutputInfo> {
        let index = *self.bindings.get(slot.index())?;
        self.layout.outputs.get_mut(index?)
    }
}

/// Mirrors `flowshot_core::geometry`'s totality rule: an invalid scale falls
/// back to `1.0` instead of dividing by zero (validated construction happens
/// in `OutputInfo::new`, but its fields are public).
fn effective_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// `f64::max`/`min` ignore a NaN operand, so non-finite input collapses to
/// `lo` deterministically; unlike `f64::clamp` this never panics.
fn clamp_axis(value: f64, lo: f64, hi: f64) -> f64 {
    value.max(lo).min(hi)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )]

    use super::*;
    use flowshot_core::geometry::Transform;

    const TOL: f64 = 1e-9;

    fn output(connector: &str, rect: LogicalRect, size: PhysicalSize, scale: f64) -> OutputInfo {
        OutputInfo::new(connector, connector, rect, size, scale, Transform::Normal)
            .expect("test output must be valid")
    }

    /// Single 1920x1080 @ 1x anchored at the origin.
    fn single_1080p() -> OutputLayout {
        OutputLayout::new(vec![output(
            "DP-1",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            1.0,
        )])
    }

    /// Dual mixed-DPI: DP-1 1920x1080 @ 1x at (0,0); DP-2 3840x2160 @ 2x
    /// at logical (1920,0) -> global logical span 3840x1080.
    fn dual_mixed() -> OutputLayout {
        OutputLayout::new(vec![
            output(
                "DP-1",
                LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
                PhysicalSize::from_raw(1920, 1080),
                1.0,
            ),
            output(
                "DP-2",
                LogicalRect::from_raw(1920.0, 0.0, 1920.0, 1080.0),
                PhysicalSize::from_raw(3840, 2160),
                2.0,
            ),
        ])
    }

    fn dual_router() -> InputRouter {
        InputRouter::new(dual_mixed(), vec![0, 1])
    }

    fn approx_eq(a: LogicalPoint, bx: f64, by: f64) -> bool {
        (a.x.0 - bx).abs() <= TOL && (a.y.0 - by).abs() <= TOL
    }

    #[test]
    fn to_global_single_monitor_is_identity_at_scale_one() {
        let router = InputRouter::new(single_1080p(), vec![0]);
        let global = router.to_global(WindowSlot::new(0), 100.0, 200.0).unwrap();
        assert_eq!((global.x.0, global.y.0), (100.0, 200.0));
    }

    #[test]
    fn to_global_second_monitor_applies_offset_then_scale() {
        let router = dual_router();
        // DP-2 @ 2x: local physical (200,400) -> logical (100,200) + origin (1920,0).
        let global = router.to_global(WindowSlot::new(1), 200.0, 400.0).unwrap();
        assert_eq!((global.x.0, global.y.0), (2020.0, 200.0));
    }

    #[test]
    fn spanning_drag_extends_beyond_window_bounds_into_neighbor() {
        // Implicit pointer grab: drag started on DP-1 (slot 0, 1920 px wide);
        // the cursor is now 300 physical px beyond DP-1's right edge, but the
        // event still arrives at slot 0. Global mapping must place it inside
        // DP-2's logical range - never clamp to the source window.
        let router = dual_router();
        let global = router.to_global(WindowSlot::new(0), 2220.0, 500.0).unwrap();
        assert_eq!((global.x.0, global.y.0), (2220.0, 500.0));
        let owner = router
            .layout()
            .output_at(global)
            .expect("point is in layout");
        assert_eq!(owner.connector, "DP-2");
    }

    #[test]
    fn spanning_drag_accepts_negative_local_coordinates() {
        // Dragging past the top-left edge yields negative surface-local
        // positions under the implicit grab; they map linearly, unclamped.
        let router = InputRouter::new(
            OutputLayout::new(vec![output(
                "DP-2",
                LogicalRect::from_raw(1920.0, 0.0, 1920.0, 1080.0),
                PhysicalSize::from_raw(3840, 2160),
                2.0,
            )]),
            vec![0],
        );
        let global = router.to_global(WindowSlot::new(0), -400.0, -20.0).unwrap();
        assert_eq!((global.x.0, global.y.0), (1720.0, -10.0));
    }

    #[test]
    fn clamped_mapping_stays_inside_layout_bounds() {
        let router = dual_router();
        let clamped = router
            .to_global_clamped(WindowSlot::new(0), 5000.0, 4000.0)
            .unwrap();
        // Union bounds: (0,0)..(3840,1080), closed interval.
        assert_eq!((clamped.x.0, clamped.y.0), (3840.0, 1080.0));
        let unclamped = router
            .to_global(WindowSlot::new(0), 5000.0, 4000.0)
            .unwrap();
        assert_eq!((unclamped.x.0, unclamped.y.0), (5000.0, 4000.0));
    }

    #[test]
    fn clamp_point_collapses_non_finite_to_bounds_minimum() {
        let router = dual_router();
        let clamped = router.clamp_point(LogicalPoint::from_raw(f64::NAN, f64::INFINITY));
        assert_eq!((clamped.x.0, clamped.y.0), (0.0, 1080.0));
    }

    #[test]
    fn to_local_inverts_to_global_on_both_monitors() {
        let router = dual_router();
        for slot_index in [0usize, 1] {
            let slot = WindowSlot::new(slot_index);
            for (x, y) in [(0.0, 0.0), (333.0, 777.0), (1919.0, 1079.0)] {
                let global = router.to_global(slot, x, y).unwrap();
                let (lx, ly) = router.to_local(slot, global).unwrap();
                assert!(
                    (lx - x).abs() <= TOL && (ly - y).abs() <= TOL,
                    "slot {slot_index} ({x},{y}) -> ({lx},{ly})"
                );
            }
        }
    }

    #[test]
    fn to_local_of_neighbor_point_lands_outside_surface() {
        // A global point over DP-2 expressed in DP-1's surface space exceeds
        // DP-1's width - expected and needed for spanning-selection drawing.
        let router = dual_router();
        let (lx, ly) = router
            .to_local(WindowSlot::new(0), LogicalPoint::from_raw(2020.0, 200.0))
            .unwrap();
        assert!((lx - 2020.0).abs() <= TOL && (ly - 200.0).abs() <= TOL);
        assert!(lx > 1920.0);
    }

    #[test]
    fn unknown_slot_routes_to_none_without_panic() {
        let router = dual_router();
        assert_eq!(router.to_global(WindowSlot::new(7), 10.0, 10.0), None);
        assert_eq!(
            router.to_local(WindowSlot::new(7), LogicalPoint::from_raw(0.0, 0.0)),
            None
        );
        assert!(router.output_for(WindowSlot::new(7)).is_none());
    }

    #[test]
    fn non_finite_local_position_is_rejected() {
        let router = dual_router();
        assert_eq!(router.to_global(WindowSlot::new(0), f64::NAN, 0.0), None);
        assert_eq!(
            router.to_global(WindowSlot::new(0), 0.0, f64::INFINITY),
            None
        );
    }

    #[test]
    fn update_surface_size_refines_logical_rect_for_rotated_output() {
        // Monitor reported a pre-transform 1920x1080 mode; the fullscreen
        // window's first Resized reveals the post-transform 1080x1920 surface.
        let mut router = InputRouter::new(single_1080p(), vec![0]);
        router
            .update_surface_size(WindowSlot::new(0), PhysicalSize::from_raw(1080, 1920))
            .unwrap();
        let output = router.output_for(WindowSlot::new(0)).unwrap();
        assert_eq!(output.buffer_size(), PhysicalSize::from_raw(1080, 1920));
        assert!(approx_eq(
            LogicalPoint::from_raw(output.logical_rect.width.0, output.logical_rect.height.0),
            1080.0,
            1920.0
        ));
        // Origin is preserved.
        assert_eq!(
            (output.logical_rect.x.0, output.logical_rect.y.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn update_scale_recomputes_logical_size_and_rejects_invalid() {
        let mut router = InputRouter::new(single_1080p(), vec![0]);
        router.update_scale(WindowSlot::new(0), 2.0).unwrap();
        let output = router.output_for(WindowSlot::new(0)).unwrap();
        assert_eq!(output.scale, 2.0);
        assert!((output.logical_rect.width.0 - 960.0).abs() <= TOL);
        assert!(matches!(
            router.update_scale(WindowSlot::new(0), 0.0),
            Err(GeometryError::InvalidScale(0.0))
        ));
        // Unbound slot: silent no-op.
        router.update_scale(WindowSlot::new(9), 3.0).unwrap();
    }

    #[test]
    fn replace_layout_rebinds_by_connector_name() {
        let mut router = dual_router();
        // Monitors re-ordered after a hotplug: DP-2 is now output 0.
        let reordered = OutputLayout::new(vec![
            router.layout().outputs[1].clone(),
            router.layout().outputs[0].clone(),
        ]);
        router.replace_layout(reordered);
        let global = router.to_global(WindowSlot::new(1), 200.0, 400.0).unwrap();
        assert_eq!((global.x.0, global.y.0), (2020.0, 200.0));
        // A vanished connector unbinds its slot.
        router.replace_layout(single_1080p());
        assert!(router.output_for(WindowSlot::new(0)).is_some());
        assert!(router.output_for(WindowSlot::new(1)).is_none());
    }

    #[test]
    fn install_replaces_bindings_wholesale() {
        let mut router = dual_router();
        router.install(single_1080p(), vec![0]);
        assert_eq!(router.window_count(), 1);
        assert_eq!(router.to_global(WindowSlot::new(1), 0.0, 0.0), None);
    }
}
