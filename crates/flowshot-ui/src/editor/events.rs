//! The editor's input surface: the event entry points
//! [`OverlayCore`](crate::OverlayCore) feeds from its route funnel. Split
//! from the facade so [`super::EditorState`] keeps owning state and
//! accessors while this file owns the event semantics - the same discipline
//! as `selection/events.rs` - and split again at the 250-LOC ceiling into
//! this key/wheel half and the [`pointer`] child (press/move/release).
//!
//! Routing follows the exact Flameshot priority chain via [`super::routing`];
//! these files execute the decisions: draw-session lifecycle, edit-widget
//! commits, object selection and its atomic move drag, the
//! digit/wheel size adjusters, and the undo/redo/delete scene ops.

mod pointer;

use flowshot_core::geometry::LogicalPoint;
use winit::keyboard::KeyCode;

use super::keys::{AidToggle, ZOrderAction, digit_for};
use super::size::stepped;
use super::tool::{EditorContext, Tool};
use super::types::{EditorEnv, EditorUpdate};
use super::{EditorState, kind::ToolKind};

impl EditorState {
    /// A key press (auto-repeat feeds digits and undo/redo like Flameshot's
    /// shortcut repeat, but never re-toggles tools). `text` is the winit
    /// `KeyEvent.text` payload (the edit-session input); while an
    /// edit widget is active every non-Escape key belongs to the session
    /// (`editing_key_press`) and the normal key map is
    /// skipped - typing never toggles tools or resizes.
    pub fn key_press(
        &mut self,
        env: &EditorEnv,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
    ) -> EditorUpdate {
        if let Some(update) = self.editing_key_press(env, code, repeat, text) {
            return update;
        }
        let ctrl = env.modifiers.control_key();
        let shift = env.modifiers.shift_key();
        let plain = !ctrl && !shift && !env.modifiers.alt_key() && !env.modifiers.super_key();
        if ctrl && !env.modifiers.alt_key() && !env.modifiers.super_key() {
            if shift && self.shortcuts.is_redo(code) {
                let (changed, selection) = self.redo();
                return EditorUpdate {
                    consumed: true,
                    effects: Vec::new(),
                    changed,
                    restore_selection: selection,
                    resized: false,
                };
            }
            if !shift && self.shortcuts.is_undo(code) {
                let (changed, selection) = self.undo();
                return EditorUpdate {
                    consumed: true,
                    effects: Vec::new(),
                    changed,
                    restore_selection: selection,
                    resized: false,
                };
            }
        }
        if plain {
            if let Some(digit) = digit_for(code) {
                let size = self.digits.digit(digit, env.now);
                self.apply_size(size);
                return EditorUpdate::eaten_resized();
            }
            if !repeat && let Some(kind) = self.shortcuts.tool_for_key(code) {
                self.toggle_tool(kind);
                return EditorUpdate::eaten(true);
            }
            // Aid toggles (magnifier/grid): rebindable slots in the key
            // map - the aid-indicator chips read the same bindings, so a
            // rebind is reflected in both the dispatch and the display.
            if !repeat && let Some(aid) = self.shortcuts.aid_for_key(code) {
                match aid {
                    AidToggle::Magnifier => self.toggle_magnifier(),
                    AidToggle::Grid => self.toggle_grid(),
                }
                return EditorUpdate::eaten(true);
            }
            // Z-order keys ship UNBOUND (panel-driven); when
            // rebound, a duplicate binding loses to the tool key above and
            // auto-repeat never stacks journal entries.
            if !repeat && let Some(action) = self.shortcuts.z_for_key(code) {
                let moved = match action {
                    ZOrderAction::Raise => self.raise_selected(),
                    ZOrderAction::Lower => self.lower_selected(),
                };
                return EditorUpdate::eaten(moved);
            }
        }
        if code == KeyCode::Delete && self.selected.is_some() {
            return EditorUpdate::eaten(self.delete_selected());
        }
        EditorUpdate::pass()
    }

    /// A wheel angle-delta (the shell converts winit's line/pixel deltas):
    /// the active tool gets first refusal on the thresholded step (the
    /// counter bubble increment), otherwise the tool size moves ±1.
    pub fn wheel(&mut self, env: &EditorEnv, delta: i32) -> EditorUpdate {
        if env.picker_visible {
            return EditorUpdate::eaten(false);
        }
        let Some(step) = self.wheel_acc.wheel(delta) else {
            return EditorUpdate::eaten(false);
        };
        let at = env.mouse.unwrap_or_default();
        let by_tool = self
            .with_ctx(env, at, |ctx, tool| tool.wheel(ctx, step))
            .unwrap_or(false);
        if by_tool {
            return EditorUpdate::eaten(true);
        }
        let next = stepped(self.sizes.get(self.active_kind), step);
        self.apply_size(next);
        EditorUpdate::eaten_resized()
    }

    /// Runs `run` with the per-event context and the active tool (split
    /// borrow: the context borrows the editor's data fields while the tool
    /// is mutably borrowed - one function, disjoint fields).
    pub(super) fn with_ctx<T>(
        &mut self,
        env: &EditorEnv,
        mouse: LogicalPoint,
        run: impl FnOnce(&EditorContext<'_>, &mut Box<dyn Tool>) -> T,
    ) -> Option<T> {
        let Self {
            tool,
            frame,
            config,
            color,
            sizes,
            scene,
            active_kind,
            ..
        } = self;
        let tool = tool.as_mut()?;
        let ctx = EditorContext {
            frame: frame.as_ref(),
            selection: env.selection,
            color: *color,
            tool_size: sizes.get(*active_kind),
            mouse,
            modifiers: env.modifiers,
            circle_count: scene.next_counter_value(),
            config: &*config,
        };
        Some(run(&ctx, tool))
    }

    /// Applies a size to the active dispatch slot, notifies the tool, and
    /// logs the stable `size` field (the QA digit-clip assert).
    pub(super) fn apply_size(&mut self, size: u32) {
        self.sizes.set(self.active_kind, size);
        let size = self.sizes.get(self.active_kind);
        if let Some(tool) = self.tool.as_mut() {
            tool.on_size_changed(size);
        }
        tracing::info!(
            target: "flowshot_ui::editor",
            kind = self.active_kind.map_or("shared", ToolKind::id),
            size,
            "tool size"
        );
    }
}
