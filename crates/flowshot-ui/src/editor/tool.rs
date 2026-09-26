//! The tool contract (plan todo 20, draft F27 tool model).
//!
//! [`Tool`] is the clean-room equivalent of Flameshot's `CaptureTool`
//! pure-virtual contract (`isValid`/`copy`/`process`/`paintMousePreview`/
//! `drawStart`/`drawMove`/`drawEnd`/`pressed`/`boundingRect`/`count`/
//! `onColor|SizeChanged`): every annotation tool (todos 21-27) implements it,
//! receives the per-event [`EditorContext`] (the F27 `CaptureContext`
//! equivalent), paints through the renderer-agnostic
//! [`PaintSink`](flowshot_core::scene::PaintSink) - the SAME sink the scene
//! objects paint through, bridged to the render [`DisplayList`] by
//! [`super::paint`] - and reports its committed object at stroke end.
//!
//! Coordinate contract: tools live in SCENE space = GLOBAL LOGICAL pixels
//! (the selection engine's space, f32 precision via the scene types). The
//! paint bridge converts to each window's local physical pixels with the
//! output's own scale - tools never touch physical coordinates.
//!
//! Lifecycle (Flameshot parity): a FRESH instance is created from the
//! registry per stroke press (Flameshot `m_activeButton->tool()->copy()`);
//! the instance survives the stroke for the mouse preview and any edit
//! widget (todo 22), and is replaced by the next press.

use flowshot_core::config::{Config, EditorConfig, ToolsConfig};
use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{Color, PaintSink, ToolObject, ToolObjectData};
use winit::event::{Ime, MouseButton};
use winit::keyboard::{KeyCode, ModifiersState};

use super::effect::PixelEffect;
use super::kind::ToolKind;

/// The frozen-frame pixel view tools sample from (F27 `CaptureContext`
/// `screenshot`/`origScreenshot` equivalent - the secure pixelate of todo 23
/// reads the ORIGINAL frame through this, never a GPU texture).
///
/// Row-major upright `RGBA8888`, one view per captured output; the shell
/// installs it through [`OverlayCore::install_frame`](crate::OverlayCore::install_frame).
#[derive(Debug, Clone, PartialEq)]
pub struct FramePixels {
    /// Row-major `RGBA8888` pixels (`width * height * 4`).
    pub rgba: Vec<u8>,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// The output's scale factor (physical px per logical px).
    pub scale: f64,
    /// Global logical position of the frame's top-left corner.
    pub origin: LogicalPoint,
}

/// The config projection tools receive by reference (F27 `CaptureContext`
/// config access; the grouped `[editor]` + `[tools.*]` schema of todo 2).
///
/// `mouse_preview` carries the `showMousePreview` behavior (default on).
/// ORPHAN NOTE: the plan's todo-2 authoritative-naming rule lists
/// `showMousePreview -> [editor].mouse_preview` as an ADDED key, but the
/// landed core config does not carry it yet - until it does, this flag is
/// editor-side state seeded to the spec default (`true`), recorded in the
/// notepad for the config write-back todo.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorTools {
    /// The `[editor]` group.
    pub editor: EditorConfig,
    /// The `[tools.*]` groups.
    pub tools: ToolsConfig,
    /// Whether tools paint a cursor-following preview (`showMousePreview`).
    pub mouse_preview: bool,
}

impl Default for EditorTools {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl EditorTools {
    /// Projects the editor-relevant groups out of a full config.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            editor: config.editor.clone(),
            tools: config.tools,
            mouse_preview: true,
        }
    }
}

/// The per-event context handed to every [`Tool`] call - the F27
/// `CaptureContext` equivalent (`{screenshot, selection, color, toolSize,
/// mousePos, circleCount, config}`).
#[derive(Debug, Clone, Copy)]
pub struct EditorContext<'a> {
    /// The frozen original frame, when the shell installed one (todo 23
    /// samples it; `None` on the empty overlay).
    pub frame: Option<&'a FramePixels>,
    /// The current selection in global logical space.
    pub selection: Option<LogicalRect>,
    /// The active draw color (`[editor].draw_color`, wheel/picker-updated).
    pub color: Color,
    /// The dispatched size for the active tool (todo-20 size table).
    pub tool_size: u32,
    /// The pointer position, global logical (press position during a stroke,
    /// live cursor for the preview).
    pub mouse: LogicalPoint,
    /// The keyboard modifier snapshot - the F27 drag conventions read it:
    /// Ctrl constrains the shape (line H/V/45deg, rect aspect lock, ellipse
    /// circle lock - todo 21 semantics), Shift applies the per-tool mirror/
    /// square rules. Selection-engine modifiers (Shift mirror resize, Ctrl
    /// aspect) stay in the todo-16 engine and are NOT reinterpreted here.
    pub modifiers: ModifiersState,
    /// The next circle-counter number (the scene's max+1 rule, todo 24).
    pub circle_count: u32,
    /// The config projection (font family, per-tool option groups).
    pub config: &'a EditorTools,
}

/// What the shell draws at the cursor while a tool is active (the
/// "active-tool cursor change": the system cursor is hidden overlay-wide,
/// the crosshair layer is ours - a tool may suppress it in favor of its own
/// mouse preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolCursor {
    /// The standard crosshair (default).
    #[default]
    Crosshair,
    /// No shell cursor: the tool's own preview is the cursor.
    Hidden,
}

/// A key press routed to the active edit session (the todo-22 text-editing
/// surface: while a tool edit widget is open, the editor funnel hands every
/// non-Escape key here BEFORE the normal key map - Flameshot's child-widget
/// focus parity, so typing `t` inserts text instead of toggling the tool).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditKey<'a> {
    /// The physical key.
    pub code: KeyCode,
    /// The text the key produced (winit `KeyEvent.text`; `None` for
    /// non-text keys and IME-consumed presses).
    pub text: Option<&'a str>,
    /// Whether the press is an auto-repeat.
    pub repeat: bool,
}

/// The annotation-tool contract (F27 `CaptureTool` clean-room equivalent).
///
/// Only [`Tool::kind`] is mandatory; every lifecycle method has a no-op
/// default so action-style and edit-style tools implement just their slice
/// (Flameshot's virtual defaults, same ergonomics). Draw sessions are
/// press-driven: [`Tool::draw_start`] on the routed press, [`Tool::draw_move`]
/// per motion, [`Tool::draw_end`] on release - returning `Some(object)`
/// commits it to the scene as ONE undo unit, `None` commits nothing (the
/// zero-length-drag rule of todo 21 lives in the tool's own validity check).
pub trait Tool: std::fmt::Debug + Send {
    /// Downcast support (the move-selection tool's delta access seam).
    fn as_any(&self) -> &dyn std::any::Any;

    /// Mutable downcast support (the move-selection tool's delta reset seam).
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// The stable kind this tool implements.
    fn kind(&self) -> ToolKind;

    /// A routed press with the active tool: called BEFORE
    /// [`Tool::draw_start`]; returning `true` consumes the press without
    /// opening a draw session (single-click placement, edit-widget entry -
    /// the F27 `pressed` slot).
    fn pressed(
        &mut self,
        _ctx: &EditorContext<'_>,
        _button: MouseButton,
        _at: LogicalPoint,
    ) -> bool {
        false
    }

    /// Opens a draw session at `at` (F27 `drawStart`).
    fn draw_start(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    /// Extends the active draw session to `at` (F27 `drawMove`).
    fn draw_move(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    /// Ends the draw session at `at`; the returned object (when any) is
    /// committed to the scene as one undo unit (F27 `drawEnd` + `process`).
    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        None
    }

    /// Ends the draw session at `at` as a DESTRUCTIVE pixel op (todo 23):
    /// the returned baked effect is committed to the editor's pixel-overlay
    /// layer as one undo unit (checked BEFORE [`Tool::draw_end`] - a tool
    /// implements exactly one of the two channels).
    fn draw_end_effect(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<PixelEffect> {
        None
    }

    /// Paints the in-progress shape and/or the cursor-following mouse
    /// preview into `sink` (scene space; F27 `paintMousePreview` + the
    /// live half of `paint`). Called only while drawing or while the
    /// preview is enabled.
    fn paint(&self, _ctx: &EditorContext<'_>, _sink: &mut dyn PaintSink) {}

    /// The in-progress shape's bounds, when a session is open (F27
    /// `boundingRect`; damage tracking for repaints).
    fn bounding_rect(&self) -> Option<LogicalRect> {
        None
    }

    /// Whether the in-progress shape is committable (F27 `isValid`).
    fn is_valid(&self) -> bool {
        true
    }

    /// The draw color changed (picker/config; F27 `onColorChanged`).
    fn on_color_changed(&mut self, _color: Color) {}

    /// The dispatched tool size changed (digits/wheel/panel; F27
    /// `onSizeChanged`).
    fn on_size_changed(&mut self, _size: u32) {}

    /// Whether this tool paints a cursor-following preview at all (F27
    /// `showMousePreview` per-tool half; the config half gates globally).
    fn show_mouse_preview(&self) -> bool {
        true
    }

    /// A thresholded wheel step (±1) with the tool active: returning `true`
    /// consumes it (the counter tool's bubble increment, todo 24 - F27
    /// `handleMouseWheelEvent`), `false` lets it adjust the tool size.
    fn wheel(&mut self, _ctx: &EditorContext<'_>, _step: i32) -> bool {
        false
    }

    /// The geometry of this tool's in-scene edit widget while editing (the
    /// text box of todo 22); `None` when not editing. Drives the F27
    /// right-click exception, the click-outside commit, and the Esc
    /// cascade's tool-widget stage.
    fn edit_rect(&self) -> Option<LogicalRect> {
        None
    }

    /// Commits the active edit widget (click-outside / Ctrl+Return, F27
    /// text lifecycle); the returned object becomes one undo unit.
    fn commit_edit(&mut self, _ctx: &EditorContext<'_>) -> Option<Box<dyn ToolObject>> {
        None
    }

    /// Cancels the active edit widget without committing (Esc mid-edit,
    /// todo 22 failure path).
    fn cancel_edit(&mut self) {}

    /// A key press routed to the active edit session (todo 22): returning
    /// `true` consumes it (the Flameshot child-widget focus parity - the
    /// normal key map never sees keys an edit widget ate).
    fn edit_key(&mut self, _ctx: &EditorContext<'_>, _key: EditKey<'_>) -> bool {
        false
    }

    /// A winit IME event routed to the active edit session (todo 22, the
    /// always-on model of draft D7); returning `true` consumes it.
    fn ime(&mut self, _ctx: &EditorContext<'_>, _ime: &Ime) -> bool {
        false
    }

    /// Takes over an existing committed object for in-place re-editing
    /// (todo 22: a press on a text object re-enters edit preserving the old
    /// text); returning `true` opens the edit session and the editor funnel
    /// replaces the object as ONE undo unit on commit.
    fn edit_object_data(&mut self, _ctx: &EditorContext<'_>, _data: &ToolObjectData) -> bool {
        false
    }

    /// The caret rect in global logical space while editing (the IME
    /// cursor-area source - the shell mirrors it into
    /// `Window::set_ime_cursor_area` so the IME popup anchors at the caret).
    fn caret_rect(&self) -> Option<LogicalRect> {
        None
    }

    /// The cursor shape while this tool is active.
    fn cursor(&self) -> ToolCursor {
        ToolCursor::Crosshair
    }
}
