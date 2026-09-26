//! The route funnel (plan todo 13/16/20): coordinate mapping plus the F27
//! event-routing priority - the editor sees every pointer/key/wheel event
//! FIRST and consumes per the priority chain (picker > right-click > active
//! tool > edit commit > object select); whatever it passes through belongs
//! to the selection engine (region geometry - the todo-16 contract). The
//! Esc cascade stays in the selection engine (its six-stage order is the
//! final contract); this funnel applies the popped step's editor-side
//! reaction and keeps the cascade flags in sync after every event.

use std::time::Instant;

use winit::event::{Ime, MouseButton};
use winit::keyboard::KeyCode;

use crate::editor::{EditorEffect, EditorEnv, EditorUpdate};
use crate::input::{Action, InputEvent, RouteReport};
use crate::router::WindowSlot;
use crate::selection::{Effect, EscStep, SelectionEnv, SelectionState, SelectionUpdate};

use super::{CursorTrack, OverlayCore};

/// The funnel entry (crate-internal; external injection goes through the
/// `test-drive` seam).
pub(super) fn route(core: &mut OverlayCore, slot: WindowSlot, event: &InputEvent) -> RouteReport {
    match event {
        InputEvent::PointerMoved { x, y } => core.route_motion(slot, *x, *y),
        InputEvent::PointerButton { button, pressed } => core.route_button(slot, *button, *pressed),
        InputEvent::Key {
            code,
            pressed,
            repeat,
            text,
        } => core.route_key(slot, *code, *pressed, *repeat, text.as_deref()),
        InputEvent::Wheel { delta_y } => core.route_wheel(*delta_y),
        InputEvent::Modifiers(modifiers) => {
            core.modifiers = *modifiers;
            RouteReport::default()
        }
        InputEvent::Ime(ime) => core.route_ime(ime),
    }
}

impl OverlayCore {
    fn route_motion(&mut self, slot: WindowSlot, x: f64, y: f64) -> RouteReport {
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
            actions.push(Action::Redraw(slot));
            let env = self.editor_env();
            let outcome = self.editor.pointer_move(&env, clamped);
            let consumed = outcome.consumed;
            actions.extend(editor_actions(outcome, self.router.window_count()));
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

    fn route_button(
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
            // Chrome FIRST (todo 26, Qt child-widget parity): a press on the
            // toolbar / color wheel / side panel never reaches the F27 chain,
            // and a chrome-consumed press grabs its release (the layer
            // drag-reorder lands even when the cursor drifts).
            let chrome_ate = if pressed {
                self.chrome_press(slot, button, at)
            } else {
                self.chrome_release(slot, at)
            };
            if chrome_ate {
                actions.extend(
                    (0..self.router.window_count())
                        .map(|index| Action::Redraw(WindowSlot::new(index))),
                );
                self.sync_cascade();
                return RouteReport {
                    actions,
                    ..RouteReport::default()
                };
            }
            let env = self.editor_env();
            let outcome = if pressed {
                self.editor.pointer_press(&env, button, at)
            } else {
                self.editor.pointer_release(&env, button, at)
            };
            // The wheel-open effect is applied IN the funnel (the chrome is
            // core-owned state, so the headless path owns the whole picker
            // flow; the shell's Action::ColorWheel arm only redraws).
            if outcome.effects.contains(&EditorEffect::ColorWheel) {
                self.chrome.show_color_wheel(at);
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
            self.sync_cascade();
        }
        RouteReport {
            actions,
            ..RouteReport::default()
        }
    }

    fn route_key(
        &mut self,
        slot: WindowSlot,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
        text: Option<&str>,
    ) -> RouteReport {
        // Latency span + elapsed sample feed the todo-38 keypress->map budget.
        let started = Instant::now();
        let _span =
            tracing::trace_span!("input.key_to_map", window = slot.index(), pressed, repeat)
                .entered();
        let mut actions = Vec::new();
        if pressed {
            let env = self.editor_env();
            let outcome = self.editor.key_press(&env, code, repeat, text);
            let consumed = outcome.consumed;
            actions.extend(editor_actions(outcome, self.router.window_count()));
            if !consumed {
                // Space toggles the side panel (plan todo 26) between the
                // editor (a text edit session owns the key while typing) and
                // the selection engine (which has no Space binding).
                if code == KeyCode::Space && !repeat && self.chrome_space() {
                    actions.extend(
                        (0..self.router.window_count())
                            .map(|index| Action::Redraw(WindowSlot::new(index))),
                    );
                } else {
                    let update =
                        feed_selection(self, |selection, env| selection.key_press(env, code));
                    if let Some(step) = update.esc_step {
                        self.apply_esc_step(step);
                    }
                    if update.effects.contains(&Effect::Exit) {
                        self.exit_requested = true;
                    }
                    actions.extend(update_actions(update, self.router.window_count()));
                }
            }
            self.sync_cascade();
        }
        tracing::trace!(
            target: "flowshot_ui::latency",
            ?code,
            elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            "key routed"
        );
        RouteReport {
            actions,
            ..RouteReport::default()
        }
    }

    fn route_wheel(&mut self, delta_y: i32) -> RouteReport {
        let env = self.editor_env();
        let outcome = self.editor.wheel(&env, delta_y);
        let actions = editor_actions(outcome, self.router.window_count());
        RouteReport {
            actions,
            ..RouteReport::default()
        }
    }

    /// The IME funnel (todo 22): the shared status tracking first, then the
    /// editor's active edit session (the always-on model of draft D7 -
    /// winit `Ime` events route here from every window).
    fn route_ime(&mut self, ime: &Ime) -> RouteReport {
        self.apply_ime(ime);
        let env = self.editor_env();
        let outcome = self.editor.ime_event(&env, ime);
        let actions = editor_actions(outcome, self.router.window_count());
        RouteReport {
            actions,
            ..RouteReport::default()
        }
    }

    /// The editor's per-event environment (the selection rect, modifier
    /// snapshot, event clock, picker-cascade flag, and live cursor).
    fn editor_env(&self) -> EditorEnv {
        EditorEnv {
            selection: self.selection.rect(),
            modifiers: self.modifiers,
            now: Instant::now(),
            picker_visible: self.selection.cascade().picker_visible(),
            mouse: self.cursor.map(|cursor| cursor.clamped),
        }
    }

    /// Applies the editor-side reaction to a popped Esc-cascade step (the
    /// cascade order itself is the selection engine's contract). Any step
    /// cancels a chrome grab first: an Esc mid-layer-drag must not let a
    /// later release land the reorder.
    fn apply_esc_step(&mut self, step: EscStep) {
        self.chrome.cancel_grab();
        match step {
            EscStep::DeselectTool => self.editor.deactivate_tool(),
            EscStep::DeselectObject => self.editor.deselect_object(),
            EscStep::DeleteToolWidget => self.editor.delete_tool_widget(),
            // The panel/picker stages are chrome-owned (todo 26).
            EscStep::HidePanel => self.chrome.hide_panel(),
            EscStep::HidePicker => self.chrome.hide_color_wheel(),
            // Close is the Exit effect the selection engine already emitted.
            EscStep::Close => {}
        }
    }

    /// Writes the chrome + editor occupancy into the cascade seam (stages
    /// 1/2/4 from the editor, 3/5 from the chrome - the funnel is the only
    /// writer). Crate-public: tests sync after driving the API directly.
    pub(crate) fn sync_cascade(&mut self) {
        let panel = self.chrome.panel_shown(&self.editor);
        let picker = self.chrome.color_wheel.visible;
        let (editor, cascade) = (&self.editor, self.selection.cascade_mut());
        editor.sync_cascade(cascade);
        cascade.set_panel_visible(panel);
        cascade.set_picker_visible(picker);
    }
}

/// Runs one selection-engine call with the freshly built environment
/// (the env is `Copy`, so the immutable borrow ends before the engine's
/// mutable borrow starts).
fn feed_selection(
    core: &mut OverlayCore,
    call: impl FnOnce(&mut SelectionState, &SelectionEnv) -> SelectionUpdate,
) -> SelectionUpdate {
    let env = SelectionEnv {
        bounds: core.router.layout().union_bounds(),
        modifiers: core.modifiers,
        now: Instant::now(),
    };
    call(&mut core.selection, &env)
}

/// Translates a selection update into shell actions: the mapped effects,
/// plus a redraw of EVERY window when the selection geometry or HUD
/// changed - a spanning selection (or a HUD anchored to it) can live on any
/// monitor, not just the event's window.
fn update_actions(update: SelectionUpdate, window_count: usize) -> Vec<Action> {
    let mut actions: Vec<Action> = update.effects.into_iter().map(Action::from).collect();
    if update.changed {
        actions.extend((0..window_count).map(|index| Action::Redraw(WindowSlot::new(index))));
    }
    actions
}

/// The editor twin of [`update_actions`]: scene visuals span windows exactly
/// like the selection does.
fn editor_actions(update: EditorUpdate, window_count: usize) -> Vec<Action> {
    let mut actions: Vec<Action> = update.effects.into_iter().map(Action::from).collect();
    if update.changed {
        actions.extend((0..window_count).map(|index| Action::Redraw(WindowSlot::new(index))));
    }
    actions
}
