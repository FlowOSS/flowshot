//! The chrome state: the floating-widget owner the route
//! funnel consults BEFORE the F27 editor chain (Qt child-widget parity - a
//! press on the toolbar / color wheel / side panel never reaches the scene)
//! and the shell paints after the selection chrome.
//!
//! Visibility contracts:
//! - the color wheel opens on the editor's right-click effect (the funnel
//!   applies [`EditorEffect::ColorWheel`](crate::editor::EditorEffect) by
//!   calling [`ChromeState::show_color_wheel`]) and occupies Esc-cascade
//!   stage 5 while visible;
//! - the side panel toggles with Space ([`ChromeState::toggle_panel`], the
//!   funnel's chrome key seam), is hard-gated by `[editor].side_panel`, and
//!   occupies Esc-cascade stage 3 while shown;
//! - a chrome-consumed press GRABS the release (implicit-grab parity), so
//!   the layer drag-reorder completes even when the cursor drifts, and an
//!   Esc cascade step cancels the grab ([`ChromeState::cancel_grab`]).
//!
//! The draw-color sink is the F27 "drawColor persists to TOML on change"
//! seam: the binary layer installs a writer; the lib stays pure.

mod controls;
mod input;

use std::time::Instant;

use crate::editor::EditorState;
use crate::input::Action;
use crate::render::{DisplayList, TextureId, f32_from_f64};
use crate::selection::SelectionState;
use flowshot_core::config::UiConfig;
use flowshot_core::geometry::OutputInfo;
use flowshot_core::tokens::DesignTokens;

use super::aids::AidsInput;
use super::motion::ChromeMotion;
use super::{ColorWheel, SizeHud, Toolbar, side_panel, toolbar::ToolbarButton};

/// The draw-color persistence callback (F27: a wheel pick writes
/// `[editor].draw_color` back to the TOML; the binary layer owns the path).
pub type DrawColorSink = Box<dyn Fn(&str) + Send + 'static>;

/// The armed layer drag-reorder (press on a row armed it; the release row
/// is the drop target).
#[derive(Debug, Clone, Copy)]
struct LayerDrag {
    from_z: usize,
}

/// The state of the editor chrome.
pub struct ChromeState {
    pub(crate) toolbar: Toolbar,
    pub(crate) color_wheel: ColorWheel,
    pub(crate) hud: SizeHud,
    pub(crate) motion: ChromeMotion,
    pub(crate) aids: AidsInput,
    tokens: DesignTokens,
    panel_visible: bool,
    pub(crate) grabbed: bool,
    layer_drag: Option<LayerDrag>,
    draw_color_sink: Option<DrawColorSink>,
    pending_actions: Vec<Action>,
}

impl std::fmt::Debug for ChromeState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChromeState")
            .field("toolbar", &self.toolbar)
            .field("color_wheel", &self.color_wheel)
            .field("hud", &self.hud)
            .field("motion", &self.motion)
            .field("aids", &self.aids)
            .field("tokens", &self.tokens)
            .field("panel_visible", &self.panel_visible)
            .field("grabbed", &self.grabbed)
            .field("layer_drag", &self.layer_drag)
            .field("draw_color_sink", &self.draw_color_sink.is_some())
            .field("pending_actions", &self.pending_actions)
            .finish()
    }
}

impl ChromeState {
    /// Creates a chrome state seeded with the default config projection
    /// (the default toolbar button order; [`Self::configure`] supersedes).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a new config projection: the toolbar button order (plan:
    /// "button order = config `buttons` list") and the palette tokens the
    /// chrome paints with. The chrome token set is the LIVE `[ui]`
    /// projection every theme-pass consumer reads (selection engine,
    /// crosshair, backdrop dim) - `dim_opacity` rides along even though
    /// the chrome itself never paints the dim layer.
    pub fn configure(&mut self, config: &UiConfig) {
        self.tokens.palette.accent.clone_from(&config.accent_color);
        self.tokens
            .palette
            .contrast
            .clone_from(&config.contrast_color);
        self.tokens.palette.dim_opacity = config.dim_opacity;
        self.toolbar.buttons = config
            .toolbar_buttons
            .iter()
            .map(|id| ToolbarButton::from_id(id))
            .collect();
        self.motion
            .configure(&self.tokens, self.toolbar.buttons.len(), Instant::now());
        tracing::info!(
            target: "flowshot_ui::chrome",
            buttons = config.toolbar_buttons.join(","),
            "chrome configured"
        );
    }

    /// The design tokens the chrome paints and hit-tests with (single
    /// source: the shell paints through [`Self::paint_into`]).
    #[must_use]
    pub const fn tokens(&self) -> &DesignTokens {
        &self.tokens
    }

    /// Installs the draw-color persistence sink (`None` clears; F27
    /// "drawColor persists to TOML on change" - the binary layer's seam).
    pub fn set_draw_color_sink(&mut self, sink: Option<DrawColorSink>) {
        self.draw_color_sink = sink;
    }

    /// Shows the color wheel centered at the given position.
    pub fn show_color_wheel(&mut self, at: flowshot_core::geometry::LogicalPoint) {
        self.color_wheel.visible = true;
        self.color_wheel.position = at;
    }

    /// Hides the color wheel (Esc cascade stage 5 reaction / pick /
    /// click-away - the F27 P1 "the picker consumes" rule).
    pub fn hide_color_wheel(&mut self) {
        self.color_wheel.visible = false;
    }

    /// Whether the side panel is shown: the Space toggle AND the
    /// `[editor].side_panel` config gate (Esc cascade stage 3 occupancy).
    #[must_use]
    pub fn panel_shown(&self, editor: &EditorState) -> bool {
        self.panel_visible && editor.config().editor.side_panel
    }

    /// Toggles the side panel (Space). `false` when the
    /// config gate is off - the key then falls through to the selection
    /// engine untouched.
    pub fn toggle_panel(&mut self, editor: &EditorState) -> bool {
        if !editor.config().editor.side_panel {
            return false;
        }
        self.panel_visible = !self.panel_visible;
        tracing::info!(
            target: "flowshot_ui::chrome",
            visible = self.panel_visible,
            "panel toggled"
        );
        true
    }

    /// Hides the side panel (Esc cascade stage 3 reaction).
    pub fn hide_panel(&mut self) {
        self.panel_visible = false;
    }

    /// Flashes the size HUD (the funnel's digits/wheel reaction; the
    /// Flameshot `setToolSize` -> `NotifierBox::showMessage` parity).
    pub fn show_size_hud(&mut self, now: Instant) {
        self.hud.show(now);
        tracing::debug!(
            target: "flowshot_ui::chrome",
            visible = true,
            "size notifier"
        );
    }

    /// Hides the size HUD.
    pub fn hide_size_hud(&mut self) {
        self.hud.hide();
    }

    /// Whether the size notifier is currently flashing.
    #[must_use]
    pub const fn size_hud_visible(&self) -> bool {
        self.hud.visible()
    }

    /// The size-HUD deadline flip (the core tick): `true` when the box
    /// hid and every window must redraw.
    pub(crate) fn hud_tick(&mut self, now: Instant) -> bool {
        self.hud.tick(now)
    }

    /// The size-HUD auto-hide deadline (the event-loop wake).
    #[must_use]
    pub(crate) fn hud_wake(&self) -> Option<Instant> {
        self.hud.wake()
    }

    /// Drops any chrome implicit grab (Esc mid-drag: a pending layer
    /// reorder must not land on a later release).
    pub fn cancel_grab(&mut self) {
        self.grabbed = false;
        self.layer_drag = None;
        self.aids.press = None;
    }

    /// Drains the toolbar's pending capture-completing actions (the
    /// funnel surfaces them in the route report; the chrome itself
    /// never executes them).
    pub(crate) fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Appends the chrome visuals to `list` (the shell paints this after
    /// the selection chrome; the scale derives from the output). `now`
    /// evaluates the motion timeline: the same instant the frame
    /// scheduler ticks with, so offscreen harnesses render deterministic
    /// animation stills from a synthetic clock.
    pub fn paint_into(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        selection: &SelectionState,
        atlas: TextureId,
        output: &OutputInfo,
        now: Instant,
    ) {
        let scale = f32_from_f64(output.scale);
        let rect = selection.rect();
        self.toolbar.draw(
            list,
            editor,
            &self.tokens,
            scale,
            atlas,
            rect,
            output,
            &self.motion,
            now,
        );
        self.color_wheel.draw(
            list,
            editor,
            &self.tokens,
            scale,
            atlas,
            output,
            &self.motion,
            now,
        );
        if self.panel_shown(editor)
            && let Some(selection) = rect
        {
            side_panel::paint::draw(
                list,
                editor,
                &self.tokens,
                scale,
                atlas,
                selection,
                output,
                self.motion.panel_progress(now),
            );
        }
        self.hud.draw(list, editor, &self.tokens, scale, atlas);
    }

    /// The chrome motion timeline: the shell's tick advances it,
    /// the frame scheduler reads its wake, and the paint path evaluates it.
    #[must_use]
    pub const fn motion(&self) -> &ChromeMotion {
        &self.motion
    }

    /// Advances the visibility-driven transitions (called once per
    /// event-loop pass from [`crate::OverlayCore::tick`]).
    pub(crate) fn motion_tick(
        &mut self,
        now: Instant,
        selection_present: bool,
        editor_panel: bool,
    ) {
        let wheel = self.color_wheel.visible;
        self.motion
            .tick(now, selection_present, editor_panel, wheel);
    }

    /// The reduced-motion switch (motion failure QA: transitions instant).
    pub fn set_motion_reduced(&mut self, reduced: bool) {
        self.motion.set_reduced(reduced);
    }

    /// Whether any chrome transition is still moving at `now`.
    #[must_use]
    pub fn motion_active(&self, now: Instant) -> bool {
        self.motion.active_at(now)
    }

    /// The earliest chrome settle deadline after `now` (`None` at rest).
    #[must_use]
    pub fn motion_settle(&self, now: Instant) -> Option<Instant> {
        self.motion.settle_at(now)
    }

    /// The funnel's motion seam: updates the toolbar and aid-chip hover
    /// washes from the pointer position (every motion event,
    /// editor-consumed or not).
    pub(crate) fn hover(
        &mut self,
        at: flowshot_core::geometry::LogicalPoint,
        editor: &EditorState,
        selection: Option<flowshot_core::geometry::LogicalRect>,
        output: &OutputInfo,
        now: Instant,
    ) {
        let scale = f32_from_f64(output.scale);
        let index = self
            .toolbar
            .button_at(at, selection, &self.tokens, scale, output);
        self.motion.set_hover(index, now);
        self.aids_hover(at, editor, selection, output);
    }
}

impl Default for ChromeState {
    fn default() -> Self {
        let mut state = Self {
            toolbar: Toolbar::default(),
            color_wheel: ColorWheel::default(),
            hud: SizeHud::default(),
            motion: ChromeMotion::new(&DesignTokens::default(), Instant::now()),
            aids: AidsInput::default(),
            tokens: DesignTokens::default(),
            panel_visible: false,
            grabbed: false,
            layer_drag: None,
            draw_color_sink: None,
            pending_actions: Vec::new(),
        };
        state.configure(&UiConfig::default());
        state
    }
}
