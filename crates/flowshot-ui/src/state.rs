//! Headless overlay state and the central routing entry point.
//!
//! [`OverlayCore`] owns the [`InputRouter`], the selection engine,
//! the editor tool framework, and the input-derived state every
//! window shares (cursor track, IME status, exit request). It contains no
//! windowing or GPU handles, so the full input path - coordinate mapping,
//! the F27 event-routing priority, Esc teardown, IME plumbing - is
//! unit-testable headlessly through the `test-drive` seam
//! [`OverlayCore::inject_event`]. The route funnel itself lives in
//! [`route`](mod@route) (the `selection/events.rs` split discipline).

mod route;

use std::time::Instant;

use flowshot_core::geometry::LogicalPoint;
use winit::event::Ime;
use winit::keyboard::ModifiersState;

use crate::completion::{ColorPickSink, CompletionSink};
use crate::editor::{EditorState, FramePixels};
#[cfg(any(test, feature = "test-drive"))]
use crate::input::SyntheticInput;
use crate::input::{ImeStatus, InputEvent, RouteReport};
use crate::launch::LaunchState;
use crate::router::{InputRouter, WindowSlot};
use crate::selection::SelectionState;

/// Where the pointer last was, in every space the overlay needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorTrack {
    /// The window slot that received the motion (during an implicit grab this
    /// stays the drag-origin window even while the cursor is logically over a
    /// neighbor).
    pub slot: WindowSlot,
    /// Surface-local physical position (fractional px), possibly beyond the
    /// surface bounds.
    pub local_x: f64,
    /// Surface-local physical position (fractional px).
    pub local_y: f64,
    /// Global logical position, unclamped (spanning enabler).
    pub global: LogicalPoint,
    /// Global logical position clamped to the layout bounds.
    pub clamped: LogicalPoint,
}

/// Input-derived state shared by all overlay windows.
///
/// (Not `Clone`/`PartialEq`: the editor holds live `dyn Tool` instances -
/// the headless engines it owns are individually cloneable/compareable.)
pub struct OverlayCore {
    pub(crate) router: InputRouter,
    cursor: Option<CursorTrack>,
    ime: ImeStatus,
    last_commit: Option<String>,
    exit_requested: bool,
    modifiers: ModifiersState,
    pub(crate) selection: SelectionState,
    pub(crate) editor: EditorState,
    pub(crate) chrome: crate::chrome::ChromeState,
    pub(crate) launch: LaunchState,
    completion: Option<CompletionSink>,
    color_pick: Option<ColorPickSink>,
    motion_was_active: bool,
}

impl std::fmt::Debug for OverlayCore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OverlayCore")
            .field("router", &self.router)
            .field("cursor", &self.cursor)
            .field("ime", &self.ime)
            .field("last_commit", &self.last_commit)
            .field("exit_requested", &self.exit_requested)
            .field("modifiers", &self.modifiers)
            .field("selection", &self.selection)
            .field("editor", &self.editor)
            .field("chrome", &self.chrome)
            .field("launch", &self.launch)
            .field("completion", &self.completion.is_some())
            .field("color_pick", &self.color_pick.is_some())
            .field("motion_was_active", &self.motion_was_active)
            .finish()
    }
}

impl OverlayCore {
    /// Creates a core around `router`.
    #[must_use]
    pub fn new(router: InputRouter) -> Self {
        Self {
            router,
            cursor: None,
            ime: ImeStatus::Inactive,
            last_commit: None,
            exit_requested: false,
            modifiers: ModifiersState::empty(),
            selection: SelectionState::default(),
            editor: EditorState::default(),
            chrome: crate::chrome::ChromeState::default(),
            launch: LaunchState::default(),
            completion: None,
            color_pick: None,
            motion_was_active: false,
        }
    }

    /// Installs the completion sink (`None` clears): the shell calls it
    /// with the rendered export when a capture-completing gesture fires
    /// (a binary-layer seam, the `RegionSink` pattern).
    pub fn set_completion_sink(&mut self, sink: Option<CompletionSink>) {
        self.completion = sink;
    }

    /// Installs the standalone color-pick sink (`None` clears): fired when
    /// the eyedropper samples a pixel (the binary layer's `flowshot color`).
    pub fn set_color_pick_sink(&mut self, sink: Option<ColorPickSink>) {
        self.color_pick = sink;
    }

    /// The installed completion sink (shell-side invocation).
    pub(crate) const fn completion_sink(&self) -> Option<&CompletionSink> {
        self.completion.as_ref()
    }

    /// The funnel's eyedropper hook: delivers a sampled color to the
    /// standalone color-pick sink when installed.
    pub(crate) fn notify_color_pick(&self, color: flowshot_core::scene::Color) {
        if let Some(sink) = self.color_pick.as_ref() {
            sink(color);
        }
    }

    /// TEST SEAM (feature `test-drive`, the headless execution mode):
    /// delivers one completion through the installed sink exactly like the
    /// shell's `complete()` does - the headless driver renders the export
    /// offscreen and hands it in here, so the sink wiring under test is
    /// the production one.
    #[cfg(any(test, feature = "test-drive"))]
    pub fn deliver_completion(&self, completion: crate::completion::Completion) {
        if let Some(sink) = self.completion.as_ref() {
            sink(completion);
        }
    }

    /// The central input router.
    #[must_use]
    pub const fn router(&self) -> &InputRouter {
        &self.router
    }

    /// Mutable access to the router (layout refinement on resize/scale
    /// changes).
    pub fn router_mut(&mut self) -> &mut InputRouter {
        &mut self.router
    }

    /// The latest pointer track, when motion has been routed.
    #[must_use]
    pub const fn cursor(&self) -> Option<&CursorTrack> {
        self.cursor.as_ref()
    }

    /// The plumbed IME session state.
    #[must_use]
    pub const fn ime(&self) -> &ImeStatus {
        &self.ime
    }

    /// The most recent IME commit, when any (consumed by the text tool).
    #[must_use]
    pub fn last_commit(&self) -> Option<&str> {
        self.last_commit.as_deref()
    }

    /// Whether teardown has been requested (Esc on any window closes all).
    #[must_use]
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// The selection interaction engine.
    #[must_use]
    pub const fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Mutable selection access: the config/preselect seams (the launch
    /// flows and the binary layer) and the Esc-cascade stage flags (the
    /// editor and chrome layers).
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.selection
    }

    /// The editor tool framework.
    #[must_use]
    pub const fn editor(&self) -> &EditorState {
        &self.editor
    }

    /// Mutable editor access: the tool registry (the composition roots
    /// register concrete tools), config, shortcuts, and color seams.
    pub fn editor_mut(&mut self) -> &mut EditorState {
        &mut self.editor
    }

    /// The chrome state.
    #[must_use]
    pub fn chrome(&self) -> &crate::chrome::ChromeState {
        &self.chrome
    }

    /// Mutable chrome state.
    pub fn chrome_mut(&mut self) -> &mut crate::chrome::ChromeState {
        &mut self.chrome
    }

    /// Installs the frozen original frame the editor's tools sample from
    /// (the secure-pixelate input; `None` clears).
    pub fn install_frame(&mut self, frame: Option<FramePixels>) {
        self.editor.install_frame(frame);
    }

    /// Applies a new chrome config projection AND re-themes the selection
    /// engine from the projected tokens (outline/grips/HUD follow
    /// `[ui].accent_color` - the settings-apply theme pass).
    pub fn configure_chrome(&mut self, config: &flowshot_core::config::UiConfig) {
        self.chrome.configure(config);
        let tokens = self.chrome.tokens().clone();
        self.selection.retheme(&tokens);
    }

    /// The current keyboard modifier snapshot.
    #[must_use]
    pub const fn modifiers(&self) -> &ModifiersState {
        &self.modifiers
    }

    /// Evaluates time-driven state at `now`: the HUD hide deadline plus the
    /// motion timelines. `true` when any window must redraw -
    /// a HUD flip, an animation frame, or the ONE settled frame after every
    /// transition lands (so the resting state always paints before the loop
    /// returns to `ControlFlow::Wait`).
    pub fn tick(&mut self, now: Instant) -> bool {
        let hud = self.selection.tick(now);
        let size_hud = self.chrome.hud_tick(now);
        let panel = self.chrome.panel_shown(&self.editor);
        self.chrome
            .motion_tick(now, self.selection.rect().is_some(), panel);
        let active = self.motion_active(now);
        let was_active = std::mem::replace(&mut self.motion_was_active, active);
        hud || size_hud || active || was_active
    }

    /// Whether any motion timeline is still moving at `now` (chrome reveal /
    /// panel / wheel / button wash, or the selection grip hover-grow).
    #[must_use]
    pub fn motion_active(&self, now: Instant) -> bool {
        self.chrome.motion_active(now) || self.selection.motion_active(now)
    }

    /// The next instant the event loop must wake for time-driven state:
    /// the HUD countdown deadline and the earliest motion settle deadline,
    /// with running animations paced at [`crate::motion::FRAME_INTERVAL`].
    /// `None` when idle - the shell then stays in `ControlFlow::Wait` (zero
    /// CPU, the shell contract the motion pass must not break).
    #[must_use]
    pub fn wake(&self, now: Instant) -> Option<Instant> {
        let hud = [self.selection.hud_wake(), self.chrome.hud_wake()]
            .into_iter()
            .flatten()
            .min();
        if !self.motion_active(now) {
            return hud;
        }
        let settle = [
            hud,
            self.selection.motion_wake(now),
            self.chrome.motion_settle(now),
        ]
        .into_iter()
        .flatten()
        .filter(|deadline| *deadline > now)
        .min();
        let paced = now.checked_add(crate::motion::FRAME_INTERVAL)?;
        Some(settle.map_or(paced, |deadline| deadline.min(paced)))
    }

    /// The reduced-motion switch (motion failure QA): every overlay
    /// transition snaps to its target and schedules no animation frames.
    pub fn set_motion_reduced(&mut self, reduced: bool) {
        self.chrome.set_motion_reduced(reduced);
        self.selection.set_motion_reduced(reduced);
    }

    /// Routes one normalized event: maps coordinates into global logical
    /// space, walks the F27 routing priority (editor first, selection
    /// engine for what the editor passes through), and returns the effects
    /// for the shell.
    ///
    /// This is the single funnel every input source passes through - real
    /// winit events and synthetic test-drive injections alike. Crate-internal:
    /// external injection goes through the feature-gated
    /// [`Self::inject_event`] seam.
    pub(crate) fn route(&mut self, slot: WindowSlot, event: &InputEvent) -> RouteReport {
        route::route(self, slot, event)
    }

    /// TEST SEAM (Metis blocker #1 fallback): injects a
    /// synthetic event through the exact production routing path, so mouse
    /// paths are QA-able without external injection tools.
    // By value for seam symmetry: the live injector (`OverlayHandle`) must
    // consume the input to send it across threads, and the headless seam
    // keeps the identical signature.
    #[allow(clippy::needless_pass_by_value)]
    #[cfg(any(test, feature = "test-drive"))]
    pub fn inject_event(&mut self, input: SyntheticInput) -> RouteReport {
        self.route(input.slot, &input.event)
    }

    fn apply_ime(&mut self, ime: &Ime) {
        match ime {
            Ime::Enabled => {
                tracing::trace!("ime enabled");
                self.ime = ImeStatus::Active;
            }
            Ime::Preedit(text, cursor) => {
                tracing::trace!(%text, ?cursor, "ime preedit");
                self.ime = ImeStatus::Preedit(text.clone());
            }
            Ime::Commit(text) => {
                tracing::debug!(%text, "ime commit");
                self.ime = ImeStatus::Active;
                self.last_commit = Some(text.clone());
            }
            Ime::Disabled => {
                tracing::trace!("ime disabled");
                self.ime = ImeStatus::Inactive;
            }
        }
    }
}

#[cfg(test)]
mod tests;
