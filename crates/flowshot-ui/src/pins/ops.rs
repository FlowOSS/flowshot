//! The pin transform operations: zoom commits, rotation, scale-factor
//! changes, and pinch application (split from [`super::interact`] for the
//! 250-LOC ceiling; both files extend the same [`PinState`] impl - the
//! todo-16 child-module pattern).

use super::event::PinEffect;
use super::pinch::PinchUpdate;
use super::spec::MARGIN;
use super::state::PinState;
use super::zoom::{
    ZoomChange, anchored_offset, clamp_scale, content_size, window_size, zoom_stepped,
};
use crate::render::{Point, f32_from_f64};

impl PinState {
    pub(super) fn rotate(&mut self, clockwise: bool) -> Vec<PinEffect> {
        self.rotation = if clockwise {
            self.rotation.clockwise()
        } else {
            self.rotation.counter_clockwise()
        };
        self.image_size = self.rotated_size();
        self.scale = clamp_scale(self.scale, self.bounds());
        self.offset = (0.0, 0.0);
        let mut effects = vec![PinEffect::Reupload];
        effects.extend(self.request_window());
        effects
    }

    pub(super) fn rotated_size(&self) -> (u32, u32) {
        if self.rotation.swaps_dimensions() {
            (self.upright_size.1, self.upright_size.0)
        } else {
            self.upright_size
        }
    }

    /// Commits `steps` wheel zoom steps anchored at the cursor (window
    /// center when the cursor is unknown).
    pub(super) fn zoom_by(&mut self, steps: i32) -> Vec<PinEffect> {
        if steps == 0 {
            return Vec::new();
        }
        let zoomed = zoom_stepped(self.scale, steps);
        self.commit_scale(zoomed, self.cursor_or_center())
    }

    pub(super) fn cursor_or_center(&self) -> (f64, f64) {
        self.cursor.unwrap_or((
            f64::from(self.target_window.0) / 2.0,
            f64::from(self.target_window.1) / 2.0,
        ))
    }

    pub(super) fn cursor_point(&self) -> Point {
        let (x, y) = self.cursor_or_center();
        Point::new(f32_from_f64(x), f32_from_f64(y))
    }

    /// Applies an absolute target scale anchored at `anchor_point`
    /// (window-local physical px): clamps, recomputes the window extent,
    /// and shifts the image offset so the anchored image point stays under
    /// the (screen-stationary) cursor across the compositor's resize.
    pub(super) fn commit_scale(
        &mut self,
        target_scale: f64,
        anchor_point: (f64, f64),
    ) -> Vec<PinEffect> {
        let new_scale = clamp_scale(target_scale, self.bounds());
        if (new_scale - self.scale).abs() < f64::EPSILON {
            return Vec::new();
        }
        let old_window = (
            f64::from(self.target_window.0),
            f64::from(self.target_window.1),
        );
        let new_window = window_size(content_size(self.image_size, new_scale), self.margin_px);
        let new_window_f = (f64::from(new_window.0), f64::from(new_window.1));
        self.offset = anchored_offset(&ZoomChange {
            cursor: anchor_point,
            margin: self.margin_px,
            offset: self.offset,
            scale: self.scale,
            new_scale,
            old_window,
            new_window: new_window_f,
            anchor: self.behavior.anchor,
        });
        self.scale = new_scale;
        // The compositor re-places the window by `delta`; the stationary
        // cursor's window-local coordinates shift by `-delta` (predicted so
        // consecutive wheel steps without motion stay anchored).
        let delta = (
            self.behavior.anchor.delta(old_window.0, new_window_f.0),
            self.behavior.anchor.delta(old_window.1, new_window_f.1),
        );
        if let Some(cursor) = &mut self.cursor {
            cursor.0 -= delta.0;
            cursor.1 -= delta.1;
        }
        self.target_window = new_window;
        vec![
            PinEffect::SetWindowSize {
                width: new_window.0,
                height: new_window.1,
            },
            PinEffect::Redraw,
        ]
    }

    /// Requests the window extent for the current scale/rotation/margin
    /// (rotate and scale-factor changes; the image stays centered).
    pub(super) fn request_window(&mut self) -> Vec<PinEffect> {
        let old_window = (
            f64::from(self.target_window.0),
            f64::from(self.target_window.1),
        );
        let new_window = window_size(content_size(self.image_size, self.scale), self.margin_px);
        if new_window == self.target_window {
            return vec![PinEffect::Redraw];
        }
        let delta = (
            self.behavior
                .anchor
                .delta(old_window.0, f64::from(new_window.0)),
            self.behavior
                .anchor
                .delta(old_window.1, f64::from(new_window.1)),
        );
        if let Some(cursor) = &mut self.cursor {
            cursor.0 -= delta.0;
            cursor.1 -= delta.1;
        }
        self.target_window = new_window;
        vec![
            PinEffect::SetWindowSize {
                width: new_window.0,
                height: new_window.1,
            },
            PinEffect::Redraw,
        ]
    }

    pub(super) fn rescale(&mut self, scale_factor: f64) -> Vec<PinEffect> {
        let factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        self.scale_factor = factor;
        self.margin_px = MARGIN * factor;
        self.menu = None;
        self.scale = clamp_scale(self.scale, self.bounds());
        self.request_window()
    }

    pub(super) fn touch(
        &mut self,
        id: u64,
        phase: winit::event::TouchPhase,
        position: (f64, f64),
    ) -> Vec<PinEffect> {
        match self.pinch.update(id, phase, position, self.scale) {
            PinchUpdate::None => Vec::new(),
            PinchUpdate::Preview { scale, midpoint } => {
                // Live preview repaints at the new scale WITHOUT a window
                // resize (compositor round-trips per touch frame would
                // thrash); the commit below resizes once.
                let clamped = clamp_scale(scale, self.bounds());
                if (clamped - self.scale).abs() < f64::EPSILON {
                    return Vec::new();
                }
                let window = (
                    f64::from(self.target_window.0),
                    f64::from(self.target_window.1),
                );
                self.offset = anchored_offset(&ZoomChange {
                    cursor: midpoint,
                    margin: self.margin_px,
                    offset: self.offset,
                    scale: self.scale,
                    new_scale: clamped,
                    old_window: window,
                    new_window: window,
                    anchor: self.behavior.anchor,
                });
                self.scale = clamped;
                vec![PinEffect::Redraw]
            }
            PinchUpdate::Commit { scale, midpoint } => {
                if (scale - self.scale).abs() < f64::EPSILON {
                    // The preview already applied the scale: the commit only
                    // syncs the window extent to it.
                    self.request_window()
                } else {
                    self.commit_scale(scale, midpoint)
                }
            }
        }
    }
}
