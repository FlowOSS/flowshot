//! Two-finger pinch tracking (Flameshot "pinch mirrors [the wheel zoom]").
//!
//! winit 0.30 exposes no touchpad pinch gesture events on Wayland (the
//! gesture family did not exist until later releases), so pinch zoom is
//! driven by touchscreen [`winit::event::Touch`] points: the distance ratio
//! between the two active fingers scales the pin around their midpoint.
//! Live movement previews the scale (repaint only); lifting a finger
//! COMMITS the last previewed scale - Flameshot's `pinchTriggered`
//! accumulate-then-commit shape (`totalScaleFactor` preview,
//! `GestureFinished` commit).

use std::collections::HashMap;

use winit::event::TouchPhase;

/// What the state machine should do with a touch update.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PinchUpdate {
    /// No pinch-relevant change.
    None,
    /// Live preview: draw at `scale` anchored at `midpoint` (no window
    /// resize until commit).
    Preview {
        /// The absolute target scale factor (start scale x distance ratio).
        scale: f64,
        /// Window-local anchor midpoint, physical px.
        midpoint: (f64, f64),
    },
    /// Commit the previewed scale (resize the window, same anchor).
    Commit {
        /// The absolute target scale factor.
        scale: f64,
        /// Window-local anchor midpoint, physical px.
        midpoint: (f64, f64),
    },
}

/// Active touch points and the in-flight pinch gesture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PinchTracker {
    points: HashMap<u64, (f64, f64)>,
    start: Option<PinchStart>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PinchStart {
    distance: f64,
    scale: f64,
    live_scale: f64,
    midpoint: (f64, f64),
}

fn finger_distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

impl PinchTracker {
    /// Feeds one touch transition; `current_scale` is the pin's committed
    /// scale (the ratio base captured when the second finger lands).
    pub fn update(
        &mut self,
        id: u64,
        phase: TouchPhase,
        position: (f64, f64),
        current_scale: f64,
    ) -> PinchUpdate {
        match phase {
            TouchPhase::Started => {
                self.points.insert(id, position);
                if self.points.len() == 2 && self.start.is_none() {
                    self.begin(current_scale);
                }
                PinchUpdate::None
            }
            TouchPhase::Moved => self.moved(id, position),
            TouchPhase::Ended | TouchPhase::Cancelled => self.lifted(id),
        }
    }

    fn moved(&mut self, id: u64, position: (f64, f64)) -> PinchUpdate {
        if self.points.insert(id, position).is_none() {
            return PinchUpdate::None;
        }
        let Some((a, b)) = self.finger_pair() else {
            return PinchUpdate::None;
        };
        let Some(start) = self.start.as_mut() else {
            return PinchUpdate::None;
        };
        let now = finger_distance(a, b);
        let ratio = if start.distance > 0.0 {
            now / start.distance
        } else {
            1.0
        };
        start.live_scale = start.scale * ratio;
        PinchUpdate::Preview {
            scale: start.live_scale,
            midpoint: start.midpoint,
        }
    }

    fn lifted(&mut self, id: u64) -> PinchUpdate {
        self.points.remove(&id);
        if self.points.len() >= 2 || self.start.is_none() {
            return PinchUpdate::None;
        }
        let start = self.start.take();
        self.points.clear();
        match start {
            Some(start) => PinchUpdate::Commit {
                scale: start.live_scale,
                midpoint: start.midpoint,
            },
            None => PinchUpdate::None,
        }
    }

    fn begin(&mut self, current_scale: f64) {
        if let Some((a, b)) = self.finger_pair() {
            let distance = finger_distance(a, b);
            if distance > 0.0 {
                self.start = Some(PinchStart {
                    distance,
                    scale: current_scale,
                    live_scale: current_scale,
                    midpoint: (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1)),
                });
            }
        }
    }

    fn finger_pair(&self) -> Option<((f64, f64), (f64, f64))> {
        let mut iter = self.points.values().copied();
        Some((iter.next()?, iter.next()?))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn pinch_previews_by_distance_ratio_and_commits_last_preview() {
        let mut tracker = PinchTracker::default();
        // Two fingers 100 px apart horizontally.
        assert_eq!(
            tracker.update(1, TouchPhase::Started, (100.0, 200.0), 1.0),
            PinchUpdate::None
        );
        assert_eq!(
            tracker.update(2, TouchPhase::Started, (200.0, 200.0), 1.0),
            PinchUpdate::None
        );
        // Spread to 150 px -> ratio 1.5.
        let update = tracker.update(2, TouchPhase::Moved, (250.0, 200.0), 1.0);
        assert!(matches!(update, PinchUpdate::Preview { scale, midpoint }
                if (scale - 1.5).abs() < 1e-9 && midpoint == (150.0, 200.0)));
        // Lifting one finger commits the previewed scale at the midpoint.
        let update = tracker.update(1, TouchPhase::Ended, (100.0, 200.0), 1.0);
        assert!(matches!(update, PinchUpdate::Commit { scale, midpoint }
                if (scale - 1.5).abs() < 1e-9 && midpoint == (150.0, 200.0)));
        // The gesture is over: further moves are inert.
        assert_eq!(
            tracker.update(2, TouchPhase::Moved, (300.0, 200.0), 1.5),
            PinchUpdate::None
        );
    }

    #[test]
    fn single_finger_never_starts_a_pinch() {
        let mut tracker = PinchTracker::default();
        tracker.update(1, TouchPhase::Started, (10.0, 10.0), 1.0);
        assert_eq!(
            tracker.update(1, TouchPhase::Moved, (40.0, 40.0), 1.0),
            PinchUpdate::None
        );
        assert_eq!(
            tracker.update(1, TouchPhase::Ended, (40.0, 40.0), 1.0),
            PinchUpdate::None
        );
    }

    #[test]
    fn commit_without_movement_keeps_the_start_scale() {
        let mut tracker = PinchTracker::default();
        tracker.update(1, TouchPhase::Started, (0.0, 0.0), 2.0);
        tracker.update(2, TouchPhase::Started, (80.0, 0.0), 2.0);
        let update = tracker.update(2, TouchPhase::Ended, (80.0, 0.0), 2.0);
        assert!(matches!(update, PinchUpdate::Commit { scale, .. } if (scale - 2.0).abs() < 1e-9));
    }
}
