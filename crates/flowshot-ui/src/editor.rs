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

mod blur;
mod editing;
mod effect;
mod events;
mod grid;
mod keys;
mod kind;
mod magnifier;
mod move_selection;
mod mutate;
mod outline;
pub(crate) mod paint;
mod pixelate;
mod properties;
mod registry;
mod routing;
mod scene_ops;
mod size;
mod tool;
mod tools;
mod types;
mod undo;
mod view;
mod zorder;

#[cfg(test)]
mod blur_tests;
#[cfg(test)]
mod pixelate_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wiring_tests;

use editing::Reedit;
use flowshot_core::config::MagnifierShape;
use flowshot_core::geometry::LogicalRect;
use flowshot_core::scene::{Color as SceneColor, Scene};
use mutate::ObjectMove;

use crate::selection::CascadeState;

use paint::parse_draw_color;

pub use effect::{EffectKind, PixelEffect, effect_texture_id};
pub use keys::{ToolShortcuts, ZOrderAction, digit_for};
pub use kind::ToolKind;
pub use magnifier::{
    ARM_ALPHA, CURSOR_OFFSET, GRID_MIN_ZOOM, MAG_PIXELS, MagnifierSample, MagnifierTexture,
    MagnifierView, RENDERED_PX, WINDOW_PX, ZOOM, magnifier_texture_id,
};
pub use outline::{DASH_OFF, DASH_ON, OBJECT_OUTLINE_INNER, OBJECT_OUTLINE_OUTER};
pub use paint::{render_color, scene_color_from_hex};
pub use registry::{ToolFactory, ToolRegistry};
pub use routing::{
    MoveTarget, PressRoute, PressTarget, ReleaseTarget, SessionRoute, route_move, route_press,
    route_release,
};
pub use size::{
    BASE_POINT_SIZE, DIGIT_RESET_DELAY, DigitAccumulator, MAX_TOOL_SIZE, MIN_TOOL_SIZE, ToolSizes,
    WHEEL_ANGLE_PER_LINE, WHEEL_THRESHOLD, WheelAccumulator, stepped,
};
pub use tool::{EditKey, EditorContext, EditorTools, FramePixels, Tool, ToolCursor};
pub use tools::{
    ArrowTool, CounterTool, EllipseTool, EyedropperTool, InvertTool, LineTool, MARKER_ALPHA,
    MarkerTool, MoveSelectionTool, PencilTool, PixelateTool, RDP_EPSILON, RectTool, SelectionTool,
    TEXT_PADDING, TextTool, register_counter_tool, register_pixelate_tools,
    register_selection_tools, register_shape_tools, register_text_tool,
};
pub use types::{EditorEffect, EditorEnv, EditorUpdate};
pub use undo::{EditorUndo, Snapshot};
pub use view::EditorView;
pub use zorder::LayerEntry;

/// The editor state machine: tool registry + active tool, the annotation
/// scene with its undo history, the object selection, the size dispatch
/// with both adjusters, and the F27 seams (frame, shortcuts, config).
// The visibility flags are independent user-facing overlay settings
// (drawing session, edit widget, grid, magnifier - the `[editor]` config
// surface); grouping them would obscure the TOML projection.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent config-driven overlay flags, the core EditorConfig pattern"
)]
#[derive(Debug)]
pub struct EditorState {
    registry: ToolRegistry,
    tool: Option<Box<dyn Tool>>,
    active_kind: Option<ToolKind>,
    drawing: bool,
    scene: Scene,
    effects: Vec<PixelEffect>,
    next_effect: u64,
    undo: EditorUndo,
    selected: Option<usize>,
    object_move: Option<ObjectMove>,
    sizes: ToolSizes,
    digits: DigitAccumulator,
    wheel_acc: WheelAccumulator,
    color: SceneColor,
    config: EditorTools,
    shortcuts: ToolShortcuts,
    frame: Option<FramePixels>,
    widget_present: bool,
    reedit: Option<Reedit>,
    grid_visible: bool,
    magnifier_visible: bool,
    magnifier_shape: MagnifierShape,
    move_selection_before: Option<Snapshot>,
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
        let undo = EditorUndo::from_undo_limit(config.editor.undo_limit);
        let sizes = ToolSizes::from_config(&config);
        let grid_visible = config.editor.grid;
        let magnifier_visible = config.editor.magnifier;
        let magnifier_shape = config.editor.magnifier_shape;
        Self {
            registry,
            tool: None,
            active_kind: None,
            drawing: false,
            scene: Scene::new(),
            effects: Vec::new(),
            next_effect: 0,
            undo,
            selected: None,
            object_move: None,
            sizes,
            digits: DigitAccumulator::default(),
            wheel_acc: WheelAccumulator::default(),
            color,
            config,
            shortcuts: ToolShortcuts::default(),
            frame: None,
            widget_present: false,
            reedit: None,
            grid_visible,
            magnifier_visible,
            magnifier_shape,
            move_selection_before: None,
        }
    }

    /// Applies a new config projection (settings seam). Runtime size
    /// adjustments reset to the config slots (Flameshot re-reads its
    /// per-tool sizes from the config on every activation).
    pub fn configure(&mut self, config: EditorTools) {
        let undo_limit = config.editor.undo_limit;
        self.color = parse_draw_color(&config.editor.draw_color);
        // Session toggles re-project from config on a settings apply (the
        // apply is authoritative over the in-session toggle): the magnifier
        // projects BOTH its config keys (todo 17), and the grid follows the
        // same pattern (todo 36 alignment - issues.md 2026-09-27 todo-17
        // follow-up: grid_visible was the lone configure() gap).
        self.magnifier_visible = config.editor.magnifier;
        self.magnifier_shape = config.editor.magnifier_shape;
        self.grid_visible = config.editor.grid;
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
    /// A new frame starts a new capture session: baked pixel effects
    /// reference the PREVIOUS frame's pixels, so they and the undo journal
    /// are dropped with it.
    pub fn install_frame(&mut self, frame: Option<FramePixels>) {
        self.frame = frame;
        self.effects.clear();
        self.undo.clear();
    }

    /// The installed frame view.
    #[must_use]
    pub const fn frame(&self) -> Option<&FramePixels> {
        self.frame.as_ref()
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
        self.cancel_reedit();
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
    /// 2, and 4; the funnel's `sync_cascade` adds the chrome-owned panel
    /// and picker stages 3/5 - todo 26).
    pub fn sync_cascade(&self, cascade: &mut CascadeState) {
        cascade.set_tool_checked(self.active_kind.is_some());
        cascade.set_object_selected(self.selected.is_some());
        cascade.set_tool_widget_present(self.editing());
    }

    /// Whether the grid overlay is visible (plan todo 27: `[editor].grid`
    /// config + toggle key).
    #[must_use]
    pub const fn grid_visible(&self) -> bool {
        self.grid_visible
    }

    /// Toggles the grid overlay visibility (the configurable key seam).
    pub fn toggle_grid(&mut self) {
        self.grid_visible = !self.grid_visible;
        tracing::info!(
            target: "flowshot_ui::editor",
            visible = self.grid_visible,
            "grid toggled"
        );
    }

    /// Sets the grid overlay visibility (config seam).
    pub fn set_grid_visible(&mut self, visible: bool) {
        self.grid_visible = visible;
    }
}
