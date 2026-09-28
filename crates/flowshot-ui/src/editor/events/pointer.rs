//! The pointer-event execution half of the funnel (split
//! from [`super`] at the 250-LOC ceiling): the F27 press/move/release
//! decisions from [`crate::editor::routing`] applied to the draw-session
//! lifecycle, the object-select press with its armed drag, and
//! the two commit channels.

use flowshot_core::geometry::LogicalPoint;
use flowshot_core::scene::ToolObject;
use winit::event::MouseButton;

use super::super::EditorState;
use super::super::effect::PixelEffect;
use super::super::kind::ToolKind;
use super::super::routing::{
    MoveTarget, PressRoute, PressTarget, ReleaseTarget, SessionRoute, route_move, route_press,
    route_release,
};
use super::super::tools;
use super::super::types::{EditorEffect, EditorEnv, EditorUpdate};

/// The two commit channels a draw release can produce (the
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
            // The picker consumes silently here; the chrome owns the wheel's input.
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
                // Eyedropper seam: if the active tool is Eyedropper and it
                // sampled a color, apply it immediately and report the pick
                // (the funnel delivers it to the color-pick sink).
                let picked = if self.active_kind == Some(ToolKind::Eyedropper)
                    && let Some(tool) = self.tool.as_ref()
                    && let Some(eyedropper) = tool.as_any().downcast_ref::<tools::EyedropperTool>()
                    && let Some(color) = eyedropper.sampled()
                {
                    self.set_color(color);
                    Some(color)
                } else {
                    None
                };
                picked.map_or_else(
                    || EditorUpdate::eaten(true),
                    |color| EditorUpdate::with(vec![EditorEffect::ColorPicked(color)], true),
                )
            }
            PressTarget::SelectObject => {
                let hit = self.select_object_at(at);
                if let Some(id) = hit {
                    self.begin_object_move(id, at);
                }
                EditorUpdate::eaten(true)
            }
            PressTarget::Selection => {
                // Flameshot P5 reset: a press that reaches the region engine
                // clears the object selection (the outline must vanish on
                // every window, hence the redraw flag).
                let had = self.selected.is_some();
                self.cancel_object_move();
                self.deselect_object();
                EditorUpdate::passing(had)
            }
        }
    }

    /// A pointer move at the layout-clamped global position `at`.
    pub fn pointer_move(&mut self, env: &EditorEnv, at: LogicalPoint) -> EditorUpdate {
        match route_move(&self.session_route(env)) {
            MoveTarget::Picker => EditorUpdate::eaten(false),
            MoveTarget::ToolDraw => {
                self.with_ctx(env, at, |ctx, tool| tool.draw_move(ctx, at));
                EditorUpdate::eaten(true)
            }
            MoveTarget::Object => EditorUpdate::eaten(self.extend_object_move(at)),
            MoveTarget::Selection => EditorUpdate::pass(),
        }
    }

    /// A pointer release at the layout-clamped global position `at`; ends
    /// the open draw session and commits its object (one undo unit), or
    /// ends the armed object drag (one move unit when it moved).
    pub fn pointer_release(
        &mut self,
        env: &EditorEnv,
        button: MouseButton,
        at: LogicalPoint,
    ) -> EditorUpdate {
        let left = button == MouseButton::Left;
        match route_release(&self.session_route(env), left) {
            ReleaseTarget::Picker => EditorUpdate::eaten(false),
            ReleaseTarget::Object => {
                self.finish_object_move();
                EditorUpdate::eaten(false)
            }
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

    /// The drag-session routing flags (picker cascade, open draw session,
    /// armed object drag).
    fn session_route(&self, env: &EditorEnv) -> SessionRoute {
        SessionRoute {
            picker_visible: env.picker_visible,
            drawing: self.drawing,
            object_move: self.object_move_armed(),
        }
    }

    /// Opens a draw session: a FRESH tool instance per stroke (Flameshot
    /// `tool()->copy()` per press), then the text-tool re-edit probe (an
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
}
