//! The selection engine's input surface (plan todo 16): the four event
//! entry points [`OverlayCore`](crate::OverlayCore) feeds from its route
//! funnel. Split from the facade so [`super::SelectionState`] keeps owning
//! state, accessors, and paint while this file owns the event semantics.

use flowshot_core::geometry::LogicalPoint;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use super::drag::{self, DragMove};
use super::types::{Effect, SelectionEnv, SelectionUpdate};
use super::{EscStep, SelectionState, cascade, keys};

impl SelectionState {
    /// A pointer press at the layout-clamped global position `at`.
    pub fn pointer_press(
        &mut self,
        env: &SelectionEnv,
        button: MouseButton,
        at: LogicalPoint,
    ) -> SelectionUpdate {
        match button {
            MouseButton::Left => {
                let mut effects = Vec::new();
                if self.consume_double_click(env.now, at) {
                    effects.push(Effect::Copy);
                }
                self.active_drag = Some(drag::begin(self.rect, at, &self.metrics));
                SelectionUpdate::with(effects, false)
            }
            MouseButton::Right => {
                // Color-wheel seam (todo 26); Flameshot shows the picker at
                // the press position - the shared cursor track carries it.
                SelectionUpdate::with(vec![Effect::ColorWheel], false)
            }
            MouseButton::Middle
            | MouseButton::Back
            | MouseButton::Forward
            | MouseButton::Other(_) => SelectionUpdate::unchanged(),
        }
    }

    /// A pointer move at the layout-clamped global position `at`.
    pub fn pointer_move(&mut self, env: &SelectionEnv, at: LogicalPoint) -> SelectionUpdate {
        let Some(active) = self.active_drag else {
            return SelectionUpdate::unchanged();
        };
        let input = DragMove {
            at,
            shift: env.modifiers.shift_key(),
            ctrl: env.modifiers.control_key(),
            metrics: &self.metrics,
            bounds: env.bounds,
        };
        match drag::advance(active, self.rect, &input) {
            drag::DragStep::Pending(next) => {
                self.active_drag = Some(next);
                SelectionUpdate::unchanged()
            }
            drag::DragStep::Changed { drag, rect } => {
                self.active_drag = Some(drag);
                self.commit_rect(Some(rect), env.now)
            }
        }
    }

    /// A pointer release at the layout-clamped global position `at`. An
    /// unpaired release (no active drag) is ignored - a release only ever
    /// completes this engine's own press.
    pub fn pointer_release(
        &mut self,
        env: &SelectionEnv,
        button: MouseButton,
        at: LogicalPoint,
    ) -> SelectionUpdate {
        if button != MouseButton::Left {
            return SelectionUpdate::unchanged();
        }
        let Some(active) = self.active_drag.take() else {
            return SelectionUpdate::unchanged();
        };
        self.commit_rect(drag::release(active, self.rect, at, &self.metrics), env.now)
    }

    /// A key press (auto-repeat counts as a press - holding an arrow keeps
    /// nudging, Flameshot's `QShortcut` repeat parity).
    pub fn key_press(&mut self, env: &SelectionEnv, code: KeyCode) -> SelectionUpdate {
        if code == KeyCode::Escape {
            let step = cascade::advance_cascade(&mut self.cascade);
            return if step == EscStep::Close {
                SelectionUpdate::with(vec![Effect::Exit], false)
            } else {
                SelectionUpdate::unchanged()
            };
        }
        let ctrl = env.modifiers.control_key();
        let shift = env.modifiers.shift_key();
        if ctrl && code == KeyCode::KeyQ {
            return SelectionUpdate::with(vec![Effect::Exit], false);
        }
        if ctrl && code == KeyCode::KeyC {
            return if self.rect.is_some() {
                SelectionUpdate::with(vec![Effect::Copy], false)
            } else {
                SelectionUpdate::unchanged()
            };
        }
        if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
            return if self.rect.is_some() {
                SelectionUpdate::with(vec![Effect::Accept], false)
            } else {
                SelectionUpdate::unchanged()
            };
        }
        if ctrl && code == KeyCode::KeyA {
            return match env.bounds {
                Some(bounds) => self.commit_rect(Some(bounds), env.now),
                None => SelectionUpdate::unchanged(),
            };
        }
        if let Some(nudge) = keys::nudge_for(code, shift, ctrl)
            && let Some(rect) = self.rect
        {
            let adjusted = keys::apply_nudge(rect, nudge, self.metrics.min_side, env.bounds);
            return self.commit_rect(Some(adjusted), env.now);
        }
        SelectionUpdate::unchanged()
    }
}
