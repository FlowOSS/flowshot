//! The editor's input surface (plan todo 20): the event entry points
//! [`OverlayCore`](crate::OverlayCore) feeds from its route funnel. Split
//! from the facade so [`super::EditorState`] keeps owning state and
//! accessors while this file owns the event semantics - the same discipline
//! as `selection/events.rs`.
//!
//! Routing follows the exact F27 priority chain via [`super::routing`];
//! this file executes the decisions: draw-session lifecycle, edit-widget
//! commits, object selection, the digit/wheel size adjusters, and the
//! undo/redo/delete scene ops.

use flowshot_core::geometry::LogicalPoint;
use flowshot_core::scene::ToolObject;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use super::effect::PixelEffect;
use super::keys::digit_for;
use super::routing::{
    MoveTarget, PressRoute, PressTarget, ReleaseTarget, route_move, route_press, route_release,
};
use super::size::stepped;
use super::tool::{EditorContext, Tool};
use super::types::{EditorEffect, EditorEnv, EditorUpdate};
use super::{EditorState, kind::ToolKind};

/// The two commit channels a draw release can produce (todo 23: the
/// destructive pixel-effect channel is checked first - a tool implements
/// exactly one of the two).
enum StrokeCommit {
    Effect(PixelEffect),
    Object(Box<dyn ToolObject>),
}

impl EditorState {
    /// A pointer press at the layout-clamped global position `at`.
    pub fn pointer_press(
        &mut self,
        env: &EditorEnv,
        button: MouseButton,
        at: LogicalPoint,
    ) -> EditorUpdate {
        let edit = self.edit_rect();
        let route = PressRoute {
            picker_visible: env.picker_visible,
            button,
            text_editing: self.editing(),
            edit_contains: edit.is_some_and(|rect| rect.contains_point(at)),
            tool_active: self.active_kind.is_some(),
            tool_is_move: self.active_kind == Some(ToolKind::Move),
            object_at: self.object_at(at).is_some(),
        };
        match route_press(&route) {
            // The picker consumes silently until todo 26 wires its input.
            PressTarget::Picker => EditorUpdate::eaten(false),
            PressTarget::ToolEdit => {
                self.with_ctx(env, at, |ctx, tool| tool.pressed(ctx, button, at));
                EditorUpdate::eaten(false)
            }
            PressTarget::ColorWheel => EditorUpdate::with(vec![EditorEffect::ColorWheel], false),
            PressTarget::CommitEdit => {
                self.commit_edit(env, at);
                EditorUpdate::eaten(true)
            }
            PressTarget::ToolDraw => {
                self.begin_stroke(env, button, at);
                EditorUpdate::eaten(true)
            }
            PressTarget::SelectObject => {
                self.select_object_at(at);
                EditorUpdate::eaten(true)
            }
            PressTarget::Selection => {
                // Flameshot P5 reset: a press that reaches the region engine
                // clears the object selection (the outline must vanish on
                // every window, hence the redraw flag).
                let had = self.selected.is_some();
                self.deselect_object();
                EditorUpdate::passing(had)
            }
        }
    }

    /// A pointer move at the layout-clamped global position `at`.
    pub fn pointer_move(&mut self, env: &EditorEnv, at: LogicalPoint) -> EditorUpdate {
        match route_move(env.picker_visible, self.drawing) {
            MoveTarget::Picker => EditorUpdate::eaten(false),
            MoveTarget::ToolDraw => {
                self.with_ctx(env, at, |ctx, tool| tool.draw_move(ctx, at));
                EditorUpdate::eaten(true)
            }
            MoveTarget::Selection => EditorUpdate::pass(),
        }
    }

    /// A pointer release at the layout-clamped global position `at`; ends
    /// the open draw session and commits its object (one undo unit).
    pub fn pointer_release(
        &mut self,
        env: &EditorEnv,
        button: MouseButton,
        at: LogicalPoint,
    ) -> EditorUpdate {
        match route_release(
            env.picker_visible,
            self.drawing,
            button == MouseButton::Left,
        ) {
            ReleaseTarget::Picker => EditorUpdate::eaten(false),
            ReleaseTarget::ToolDraw => {
                self.drawing = false;
                let committed = self
                    .with_ctx(env, at, |ctx, tool| {
                        if let Some(effect) = tool.draw_end_effect(ctx, at) {
                            return Some(StrokeCommit::Effect(effect));
                        }
                        tool.draw_end(ctx, at).map(StrokeCommit::Object)
                    })
                    .flatten();
                match committed {
                    Some(StrokeCommit::Effect(effect)) => {
                        self.commit_effect(effect);
                    }
                    Some(StrokeCommit::Object(object)) => {
                        self.commit_object(object);
                    }
                    None => {}
                }
                EditorUpdate::eaten(true)
            }
            ReleaseTarget::Selection => EditorUpdate::pass(),
        }
    }

    /// A key press (auto-repeat feeds digits and undo/redo like Flameshot's
    /// shortcut repeat, but never re-toggles tools). `text` is the winit
    /// `KeyEvent.text` payload (the todo-22 edit-session input); while an
    /// edit widget is active every non-Escape key belongs to the session
    /// ([`EditorState::editing_key_press`]) and the normal key map is
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
                return EditorUpdate::eaten(self.redo());
            }
            if !shift && self.shortcuts.is_undo(code) {
                return EditorUpdate::eaten(self.undo());
            }
        }
        if plain {
            if let Some(digit) = digit_for(code) {
                let size = self.digits.digit(digit, env.now);
                self.apply_size(size);
                return EditorUpdate::eaten(true);
            }
            if !repeat && let Some(kind) = self.shortcuts.tool_for_key(code) {
                self.toggle_tool(kind);
                return EditorUpdate::eaten(true);
            }
        }
        if code == KeyCode::Delete && self.selected.is_some() {
            return EditorUpdate::eaten(self.delete_selected());
        }
        EditorUpdate::pass()
    }

    /// A wheel angle-delta (the shell converts winit's line/pixel deltas):
    /// the active tool gets first refusal on the thresholded step (the
    /// counter bubble increment, todo 24), otherwise the tool size moves ±1.
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
        EditorUpdate::eaten(true)
    }

    /// Opens a draw session: a FRESH tool instance per stroke (Flameshot
    /// `tool()->copy()` per press), then the todo-22 re-edit probe (an
    /// object under the press the tool can take over replaces the draw
    /// start), then `pressed` (which may consume) and `draw_start`.
    fn begin_stroke(&mut self, env: &EditorEnv, button: MouseButton, at: LogicalPoint) {
        let Some(kind) = self.active_kind else {
            return;
        };
        if let Some(mut fresh) = self.registry.create(kind) {
            fresh.on_color_changed(self.color);
            fresh.on_size_changed(self.sizes.get(Some(kind)));
            self.tool = Some(fresh);
        }
        let hit = self.object_at(at);
        let hit_data = hit.and_then(|id| self.scene.get_object(id).map(ToolObject::to_data));
        let reopened = self
            .with_ctx(env, at, |ctx, tool| {
                hit_data
                    .as_ref()
                    .is_some_and(|data| tool.edit_object_data(ctx, data))
            })
            .unwrap_or(false);
        if reopened {
            if let Some(id) = hit {
                self.begin_reedit(id);
            }
            self.drawing = false;
            return;
        }
        let opened = self
            .with_ctx(env, at, |ctx, tool| {
                if tool.pressed(ctx, button, at) {
                    false
                } else {
                    tool.draw_start(ctx, at);
                    true
                }
            })
            .unwrap_or(false);
        self.drawing = opened;
        tracing::debug!(
            target: "flowshot_ui::editor",
            tool = kind.id(),
            session = opened,
            "draw start"
        );
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
