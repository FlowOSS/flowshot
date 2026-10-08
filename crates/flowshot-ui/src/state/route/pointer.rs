//! The pointer half of the route funnel: motion
//! mapping with the editor-first priority and the move-selection seam, and
//! the button funnel with the chrome-first ordering and the
//! launch-time hooks (deferred preselect on first motion, instant
//! accept on first release, region-memory persistence). Split from
//! [`super::route`] at the 250-LOC ceiling (the `events/pointer.rs`
//! precedent); the key/wheel/IME half and the shared helpers stay there.

use winit::event::MouseButton;

use crate::editor::EditorEffect;
use crate::input::{Action, RouteReport};
use crate::router::WindowSlot;
use crate::state::{CursorTrack, OverlayCore};

use super::{editor_actions, feed_selection, update_actions};

impl OverlayCore {
    pub(super) fn route_motion(&mut self, slot: WindowSlot, x: f64, y: f64) -> RouteReport {
        let _span = tracing::trace_span!("input.motion_to_map", window = slot.index()).entered();
        let global = self.router.to_global(slot, x, y);
        let clamped = global.map(|point| self.router.clamp_point(point));
        if let (Some(global), Some(clamped)) = (global, clamped) {
            self.cursor = Some(CursorTrack {
                slot,
                local_x: x,
                local_y: y,
                global,
                clamped,
            });
        }
        let mut actions = Vec::new();
        if let Some(clamped) = clamped {
            // Grip hover-grow + toolbar button wash: tracked on
            // EVERY motion regardless of editor consumption - the handles
            // and toolbar cells stay hoverable under an active tool.
            let now = std::time::Instant::now();
            self.selection.update_hover(clamped, now);
            if let Some(output) = self.router.output_for(slot) {
                let selection = self.selection.rect();
                self.chrome
                    .hover(clamped, &self.editor, selection, output, now);
            }
            // The deferred preselect (the AwaitFirstMotion contract)
            // applies on the FIRST motion, before the editor/selection see
            // it - the motion that reveals the cursor also reveals the
            // preselection in the same frame.
            actions.extend(self.launch_on_motion(clamped));
            actions.push(Action::Redraw(slot));
            let env = self.editor_env();
            let outcome = self.editor.pointer_move(&env, clamped);
            let consumed = outcome.consumed;
            actions.extend(editor_actions(outcome, self.router.window_count()));

            // Move-selection seam: when the active tool is Move and a drag is
            // in progress, apply the delta to the selection and contained objects.
            if let Some((dx, dy)) = self.editor.move_selection_delta()
                && self
                    .editor
                    .translate_selection_and_objects(&mut self.selection, dx, dy)
            {
                actions.extend(
                    (0..self.router.window_count())
                        .map(|index| Action::Redraw(WindowSlot::new(index))),
                );
            }

            if !consumed {
                let update =
                    feed_selection(self, |selection, env| selection.pointer_move(env, clamped));
                actions.extend(update_actions(update, self.router.window_count()));
            }
        }
        RouteReport {
            global_position: global,
            clamped_position: clamped,
            actions,
        }
    }

    pub(super) fn route_button(
        &mut self,
        slot: WindowSlot,
        button: MouseButton,
        pressed: bool,
    ) -> RouteReport {
        tracing::trace!(window = slot.index(), ?button, pressed, "pointer button");
        let mut actions = vec![Action::Redraw(slot)];
        // The press position is the last tracked cursor (winit always
        // delivers motion before buttons; injections must do the same).
        if let Some(cursor) = self.cursor {
            let at = cursor.clamped;
            // Chrome FIRST (Qt child-widget parity): a press on the
            // toolbar / color wheel / side panel never reaches the Flameshot chain,
            // and a chrome-consumed press grabs its release (the layer
            // drag-reorder lands even when the cursor drifts).
            let chrome_ate = if pressed {
                self.chrome_press(slot, button, at)
            } else {
                self.chrome_release(slot, at)
            };
            if chrome_ate {
                // Toolbar W5 buttons queue capture-completing actions (todo
                // 38); a completing gesture off the chrome branch persists
                // the region memory explicitly (the launch-flow contract: the
                // early return skips the main-path launch_persist).
                actions.extend(self.chrome.take_actions());
                actions.extend(
                    (0..self.router.window_count())
                        .map(|index| Action::Redraw(WindowSlot::new(index))),
                );
                self.launch_persist(&actions);
                self.sync_cascade();
                return RouteReport {
                    actions,
                    ..Default::default()
                };
            }
            let env = self.editor_env();
            let outcome = if pressed {
                self.editor.pointer_press(&env, button, at)
            } else {
                let result = self.editor.pointer_release(&env, button, at);
                // Move-selection commit seam: on release, commit the drag as
                // one undo unit (snapshot before first translation, push at release).
                if !pressed && button == MouseButton::Left {
                    self.editor.commit_move_selection();
                }
                result
            };
            // The wheel-open effect is applied IN the funnel (the chrome is
            // core-owned state, so the headless path owns the whole picker
            // flow; the shell's Action::ColorWheel arm only redraws).
            if outcome.effects.contains(&EditorEffect::ColorWheel) {
                self.chrome.show_color_wheel(at);
            }
            // The eyedropper's sample goes to the standalone color-pick sink
            // here (the binary layer's `flowshot color`); the shell's
            // Action::ColorPicked arm is a no-op.
            if let Some(color) = outcome.effects.iter().find_map(|effect| match effect {
                EditorEffect::ColorPicked(color) => Some(*color),
                EditorEffect::ColorWheel => None,
            }) {
                self.notify_color_pick(color);
            }
            let consumed = outcome.consumed;
            actions.extend(editor_actions(outcome, self.router.window_count()));
            if !consumed {
                let update = feed_selection(self, |selection, env| {
                    if pressed {
                        selection.pointer_press(env, button, at)
                    } else {
                        selection.pointer_release(env, button, at)
                    }
                });
                actions.extend(update_actions(update, self.router.window_count()));
            }
            // Launch-time flows: --instant accepts on the first
            // left release that leaves a selection; any capture-completing
            // action (Accept/Copy) persists the region memory.
            if !pressed && button == MouseButton::Left {
                actions.extend(self.launch_on_release());
            }
            self.launch_persist(&actions);
            self.sync_cascade();
        }
        RouteReport {
            actions,
            ..Default::default()
        }
    }
}
