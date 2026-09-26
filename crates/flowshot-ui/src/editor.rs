//! The editor tool framework and event routing (plan todo 20, draft F27).
//!
//! The layer bridging the scene graph (`flowshot_core::scene`, todo 4) to
//! the overlay input funnel (todo 13/16): a [`Tool`] registry every
//! annotation tool (todos 21-27) plugs into, the F27 event-routing priority
//! chain (picker > right-click > active tool > edit commit > object select >
//! selection engine), the per-tool size dispatch with the digit/wheel
//! adjusters, scene commits as single undo units, and the real producers of
//! the Esc cascade's tool/object/tool-widget stages.
//!
//! # Ownership split
//!
//! The selection engine (todo 16) owns selection GEOMETRY; the editor owns
//! the annotation scene and the active tool. [`OverlayCore`] feeds both from
//! one funnel: the editor sees every pointer/key/wheel event FIRST (the F27
//! priority) and passes through what it does not consume; the Esc cascade
//! stays in the selection engine (its six-stage order is the final contract)
//! and the editor reacts to the popped step through
//! [`OverlayCore`](crate::OverlayCore)'s step application, keeping the
//! cascade flags in sync via [`EditorState::sync_cascade`].
//!
//! # Coordinate contract
//!
//! The scene space is GLOBAL LOGICAL pixels (the selection engine's space);
//! tools never touch physical coordinates. [`EditorState::paint_into`]
//! bridges scene painting into each window's physical-px [`DisplayList`]
//! with the output's own scale (the #4871 physical-first rule), so one
//! scene spans every monitor exactly like the selection rect does.
//!
//! # Flameshot parity notes (BORROW-MODIFIED where marked)
//!
//! - a press that commits an edit widget does NOT also select an object
//!   (one press, one action),
//! - sub-threshold wheel deltas ACCUMULATE to the 60-unit step instead of
//!   Flameshot's 200ms rate limit (plan wording; a 120-unit notch still
//!   yields exactly ±1),
//! - deactivating a tool cancels its edit widget (todo 22 may refine the
//!   commit-on-switch path).

mod events;
mod keys;
mod kind;
mod paint;
mod registry;
mod routing;
mod scene_ops;
mod size;
mod tool;
mod types;

#[cfg(test)]
mod tests;

use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use flowshot_core::scene::{Color as SceneColor, Scene, UndoStack};
use winit::keyboard::ModifiersState;

use crate::render::DisplayList;
use crate::selection::CascadeState;

use paint::parse_draw_color;

pub use keys::{ToolShortcuts, digit_for};
pub use kind::ToolKind;
pub use paint::{
    DASH_OFF, DASH_ON, OBJECT_OUTLINE_INNER, OBJECT_OUTLINE_OUTER, render_color,
    scene_color_from_hex,
};
pub use registry::{ToolFactory, ToolRegistry};
pub use routing::{
    MoveTarget, PressRoute, PressTarget, ReleaseTarget, route_move, route_press, route_release,
};
pub use size::{
    BASE_POINT_SIZE, DIGIT_RESET_DELAY, DigitAccumulator, MAX_TOOL_SIZE, MIN_TOOL_SIZE, ToolSizes,
    WHEEL_ANGLE_PER_LINE, WHEEL_THRESHOLD, WheelAccumulator, stepped,
};
pub use tool::{EditorContext, EditorTools, FramePixels, Tool, ToolCursor};
pub use types::{EditorEffect, EditorEnv, EditorUpdate};

/// Everything one window's editor paint needs beyond the list and output.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorView {
    /// The live cursor (global logical) for the mouse preview; `None`
    /// before the first motion.
    pub mouse: Option<LogicalPoint>,
    /// The current selection (tool clamping context at paint time).
    pub selection: Option<LogicalRect>,
    /// The modifier snapshot (the painted preview honors the same F27
    /// constrain conventions as the committed shape).
    pub modifiers: ModifiersState,
}

/// The editor state machine: tool registry + active tool, the annotation
/// scene with its undo history, the object selection, the size dispatch
/// with both adjusters, and the F27 seams (frame, shortcuts, config).
#[derive(Debug)]
pub struct EditorState {
    registry: ToolRegistry,
    tool: Option<Box<dyn Tool>>,
    active_kind: Option<ToolKind>,
    drawing: bool,
    scene: Scene,
    undo: UndoStack,
    selected: Option<usize>,
    sizes: ToolSizes,
    digits: DigitAccumulator,
    wheel_acc: WheelAccumulator,
    color: SceneColor,
    config: EditorTools,
    shortcuts: ToolShortcuts,
    frame: Option<FramePixels>,
    widget_present: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new(EditorTools::default(), ToolRegistry::new())
    }
}

impl EditorState {
    /// Builds the editor from the config projection and a tool registry.
    #[must_use]
    pub fn new(config: EditorTools, registry: ToolRegistry) -> Self {
        let color = parse_draw_color(&config.editor.draw_color);
        let undo = UndoStack::from_undo_limit(config.editor.undo_limit);
        let sizes = ToolSizes::from_config(&config);
        Self {
            registry,
            tool: None,
            active_kind: None,
            drawing: false,
            scene: Scene::new(),
            undo,
            selected: None,
            sizes,
            digits: DigitAccumulator::default(),
            wheel_acc: WheelAccumulator::default(),
            color,
            config,
            shortcuts: ToolShortcuts::default(),
            frame: None,
            widget_present: false,
        }
    }

    /// Applies a new config projection (settings seam). Runtime size
    /// adjustments reset to the config slots (Flameshot re-reads its
    /// per-tool sizes from the config on every activation).
    pub fn configure(&mut self, config: EditorTools) {
        let undo_limit = config.editor.undo_limit;
        self.color = parse_draw_color(&config.editor.draw_color);
        self.config = config;
        self.sizes = ToolSizes::from_config(&self.config);
        self.undo
            .set_limit(usize::try_from(undo_limit).unwrap_or(usize::MAX));
        if let Some(tool) = self.tool.as_mut() {
            tool.on_color_changed(self.color);
            tool.on_size_changed(self.sizes.get(self.active_kind));
        }
    }

    /// The config projection in effect.
    #[must_use]
    pub const fn config(&self) -> &EditorTools {
        &self.config
    }

    /// Mutable registry access (the composition root registers tools before
    /// the loop starts - todo 35 / QA harnesses).
    pub fn registry_mut(&mut self) -> &mut ToolRegistry {
        &mut self.registry
    }

    /// The registry.
    #[must_use]
    pub const fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The checked tool kind, when any.
    #[must_use]
    pub const fn active_tool(&self) -> Option<ToolKind> {
        self.active_kind
    }

    /// Whether a tool is checked (Esc cascade stage 1 producer).
    #[must_use]
    pub const fn tool_active(&self) -> bool {
        self.active_kind.is_some()
    }

    /// Whether a draw session is open (press routed, release pending).
    #[must_use]
    pub const fn is_drawing(&self) -> bool {
        self.drawing
    }

    /// The dispatched size for the active tool (shared-thickness slot when
    /// none is checked).
    #[must_use]
    pub fn tool_size(&self) -> u32 {
        self.sizes.get(self.active_kind)
    }

    /// Sets the active tool's size (the todo-26 panel slider seam); digits
    /// and wheel go through the same slot dispatch.
    pub fn set_tool_size(&mut self, size: u32) {
        self.apply_size(size);
    }

    /// The active draw color.
    #[must_use]
    pub const fn color(&self) -> SceneColor {
        self.color
    }

    /// Sets the draw color (the todo-26 wheel seam; F27 `onColorChanged`).
    pub fn set_color(&mut self, color: SceneColor) {
        self.color = color;
        if let Some(tool) = self.tool.as_mut() {
            tool.on_color_changed(color);
        }
        tracing::info!(
            target: "flowshot_ui::editor",
            r = color.r,
            g = color.g,
            b = color.b,
            "draw color"
        );
    }

    /// The key map (rebindable - the todo-36 settings seam).
    #[must_use]
    pub const fn shortcuts(&self) -> &ToolShortcuts {
        &self.shortcuts
    }

    /// Mutable key-map access (configurable shortcuts).
    pub fn shortcuts_mut(&mut self) -> &mut ToolShortcuts {
        &mut self.shortcuts
    }

    /// Installs the frozen original frame tools sample from (todo 23's
    /// secure pixelate reads the ORIGINAL through this; `None` clears).
    pub fn install_frame(&mut self, frame: Option<FramePixels>) {
        self.frame = frame;
    }

    /// The installed frame view.
    #[must_use]
    pub const fn frame(&self) -> Option<&FramePixels> {
        self.frame.as_ref()
    }

    /// Checks a tool (creating it from the registry; an unregistered kind
    /// warn-logs and stays unchecked).
    pub fn activate_tool(&mut self, kind: ToolKind) {
        let Some(mut tool) = self.registry.create(kind) else {
            tracing::warn!(
                target: "flowshot_ui::editor",
                tool = kind.id(),
                "tool not registered; activation ignored"
            );
            return;
        };
        tool.on_color_changed(self.color);
        tool.on_size_changed(self.sizes.get(Some(kind)));
        if let Some(previous) = self.tool.as_mut() {
            previous.cancel_edit();
        }
        self.tool = Some(tool);
        self.active_kind = Some(kind);
        self.drawing = false;
        self.digits.reset();
        tracing::info!(target: "flowshot_ui::editor", tool = kind.id(), "tool activated");
    }

    /// Unchecks the active tool (Esc cascade stage 1 reaction). The tool's
    /// own edit session is cancelled, but a detached edit-widget flag
    /// survives until stage 4 (Flameshot parity: unchecking the tool button
    /// does NOT delete `m_toolWidget` - `deleteToolWidgetOrClose` walks the
    /// stages independently).
    pub fn deactivate_tool(&mut self) {
        let Some(kind) = self.active_kind else {
            return;
        };
        if let Some(tool) = self.tool.as_mut() {
            tool.cancel_edit();
        }
        self.tool = None;
        self.active_kind = None;
        self.drawing = false;
        self.digits.reset();
        tracing::info!(target: "flowshot_ui::editor", tool = kind.id(), "tool deactivated");
    }

    /// Activation-key semantics: re-pressing the active tool's key unchecks
    /// it (the Flameshot checkable-button toggle).
    pub fn toggle_tool(&mut self, kind: ToolKind) {
        if self.active_kind == Some(kind) {
            self.deactivate_tool();
        } else {
            self.activate_tool(kind);
        }
    }

    /// The active tool's edit-widget geometry (todo 22 producer; drives the
    /// F27 right-click exception, click-outside commit, and the Esc cascade
    /// tool-widget stage).
    #[must_use]
    pub fn edit_rect(&self) -> Option<LogicalRect> {
        self.tool.as_ref()?.edit_rect()
    }

    /// Whether an edit widget is active: the tool's own edit geometry (the
    /// todo-22 producer) OR the detached widget flag.
    #[must_use]
    pub fn editing(&self) -> bool {
        self.widget_present || self.edit_rect().is_some()
    }

    /// Sets the detached edit-widget presence (the todo-22 text-edit
    /// producer; drives the F27 right-click exception, the click-outside
    /// commit routing, and the Esc cascade's tool-widget stage).
    pub fn set_edit_widget_present(&mut self, present: bool) {
        self.widget_present = present;
    }

    /// Cancels the active edit widget without committing (Esc cascade
    /// stage 4 reaction - `deleteToolWidgetOrClose` parity).
    pub fn delete_tool_widget(&mut self) {
        self.widget_present = false;
        if let Some(tool) = self.tool.as_mut() {
            tool.cancel_edit();
        }
        tracing::debug!(target: "flowshot_ui::editor", "tool widget deleted");
    }

    /// The cursor shape while the active tool owns the cursor (the
    /// "active-tool cursor change" - the shell suppresses the crosshair for
    /// [`ToolCursor::Hidden`]).
    #[must_use]
    pub fn cursor_shape(&self) -> ToolCursor {
        self.tool
            .as_ref()
            .map_or(ToolCursor::default(), |tool| tool.cursor())
    }

    /// Writes the editor's occupancy into the Esc-cascade seam (stages 1,
    /// 2, and 4 - the panel/picker stages belong to todo 26).
    pub fn sync_cascade(&self, cascade: &mut CascadeState) {
        cascade.set_tool_checked(self.active_kind.is_some());
        cascade.set_object_selected(self.selected.is_some());
        cascade.set_tool_widget_present(self.editing());
    }

    /// Appends this window's editor visuals to `list`: the scene in paint
    /// order, the selected object's outline, and the active tool's
    /// in-progress shape / mouse preview.
    pub fn paint_into(&self, list: &mut DisplayList, output: &OutputInfo, view: EditorView) {
        let family = Some(self.config.editor.font_family.as_str());
        {
            let mut sink = paint::ListSink::new(list, output, family);
            self.scene.paint(&mut sink);
        }
        if let Some(object) = self.selected.and_then(|id| self.scene.get_object(id)) {
            paint::append_object_outline(list, output, object.bounding_rect());
        }
        let (Some(tool), Some(mouse)) = (self.tool.as_ref(), view.mouse) else {
            return;
        };
        let preview = self.config.mouse_preview && tool.show_mouse_preview();
        if !self.drawing && !preview {
            return;
        }
        let ctx = EditorContext {
            frame: self.frame.as_ref(),
            selection: view.selection,
            color: self.color,
            tool_size: self.sizes.get(self.active_kind),
            mouse,
            modifiers: view.modifiers,
            circle_count: self.scene.next_counter_value(),
            config: &self.config,
        };
        let mut sink = paint::ListSink::new(list, output, family);
        tool.paint(&ctx, &mut sink);
    }
}
