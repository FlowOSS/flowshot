//! Headless overlay state and the central routing entry point.
//!
//! [`OverlayCore`] owns the [`InputRouter`], the selection engine (todo 16),
//! the editor tool framework (todo 20), and the input-derived state every
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
#[derive(Debug)]
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

    /// The most recent IME commit, when any (consumed by the text tool,
    /// todo 22).
    #[must_use]
    pub fn last_commit(&self) -> Option<&str> {
        self.last_commit.as_deref()
    }

    /// Whether teardown has been requested (Esc on any window closes all).
    #[must_use]
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// The selection interaction engine (plan todo 16).
    #[must_use]
    pub const fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Mutable selection access: the config/preselect seams (todos 18/35)
    /// and the Esc-cascade stage flags (todos 20/26).
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.selection
    }

    /// The editor tool framework (plan todo 20).
    #[must_use]
    pub const fn editor(&self) -> &EditorState {
        &self.editor
    }

    /// Mutable editor access: the tool registry (todos 21-27/35 register
    /// concrete tools), config, shortcuts, and color seams.
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
    /// (the todo-23 secure-pixelate input; `None` clears).
    pub fn install_frame(&mut self, frame: Option<FramePixels>) {
        self.editor.install_frame(frame);
    }

    /// Applies a new chrome config projection.
    pub fn configure_chrome(&mut self, config: &flowshot_core::config::UiConfig) {
        self.chrome.configure(config);
    }

    /// The current keyboard modifier snapshot.
    #[must_use]
    pub const fn modifiers(&self) -> &ModifiersState {
        &self.modifiers
    }

    /// Evaluates time-driven selection state (the HUD hide deadline) at
    /// `now`; `true` when something changed and every window must redraw.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.selection.tick(now)
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

    /// TEST SEAM (plan todo 13, Metis blocker #1 fallback): injects a
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
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )]

    use super::*;
    use crate::input::Action;
    use flowshot_core::geometry::{LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform};
    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    /// Dual mixed-DPI fixture: DP-1 1920x1080 @ 1x at (0,0);
    /// DP-2 3840x2160 @ 2x at logical (1920,0).
    fn dual_core() -> OverlayCore {
        let make = |connector: &str, rect: LogicalRect, size: PhysicalSize, scale: f64| {
            OutputInfo::new(connector, connector, rect, size, scale, Transform::Normal)
                .expect("valid fixture output")
        };
        let layout = OutputLayout::new(vec![
            make(
                "DP-1",
                LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
                PhysicalSize::from_raw(1920, 1080),
                1.0,
            ),
            make(
                "DP-2",
                LogicalRect::from_raw(1920.0, 0.0, 1920.0, 1080.0),
                PhysicalSize::from_raw(3840, 2160),
                2.0,
            ),
        ]);
        OverlayCore::new(InputRouter::new(layout, vec![0, 1]))
    }

    #[test]
    fn inject_motion_emits_global_coords() {
        // Acceptance (plan todo 13): inject motion -> router emits global coords.
        let mut core = dual_core();
        let report = core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(1),
            200.0,
            400.0,
        ));
        let global = report.global_position.expect("slot 1 is bound");
        assert_eq!((global.x.0, global.y.0), (2020.0, 200.0));
        assert_eq!(report.clamped_position, Some(global));
        assert_eq!(report.actions, vec![Action::Redraw(WindowSlot::new(1))]);
        let cursor = core.cursor().expect("motion tracked");
        assert_eq!(cursor.slot, WindowSlot::new(1));
        assert_eq!((cursor.local_x, cursor.local_y), (200.0, 400.0));
        assert_eq!((cursor.global.x.0, cursor.global.y.0), (2020.0, 200.0));
    }

    #[test]
    fn inject_spanning_drag_motion_extends_across_window_boundary() {
        let mut core = dual_core();
        // Drag started on DP-1; implicit grab keeps delivering to slot 0 with
        // local x beyond the 1920px surface while logically over DP-2.
        let report = core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(0),
            2220.0,
            500.0,
        ));
        let global = report.global_position.unwrap();
        assert_eq!((global.x.0, global.y.0), (2220.0, 500.0));
        let owner = core.router().layout().output_at(global).expect("in layout");
        assert_eq!(owner.connector, "DP-2");
        // Clamped variant stays identical while inside the layout.
        assert_eq!(report.clamped_position, Some(global));
        // Beyond the far edge, clamped pulls back but global does not.
        let report = core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(0),
            5000.0,
            500.0,
        ));
        assert_eq!(report.global_position.unwrap().x.0, 5000.0);
        assert_eq!(report.clamped_position.unwrap().x.0, 3840.0);
    }

    #[test]
    fn inject_escape_requests_exit_from_any_window() {
        let mut core = dual_core();
        let report = core.inject_event(SyntheticInput::key_press(
            WindowSlot::new(1),
            KeyCode::Escape,
        ));
        assert_eq!(report.actions, vec![Action::Exit]);
        assert!(core.exit_requested());
        // Release and repeat do not re-trigger or clear.
        let report = core.inject_event(SyntheticInput::key_release(
            WindowSlot::new(1),
            KeyCode::Escape,
        ));
        assert!(report.actions.is_empty());
        assert!(core.exit_requested());
    }

    #[test]
    fn non_escape_keys_do_not_request_exit() {
        let mut core = dual_core();
        let report = core.inject_event(SyntheticInput::key_press(
            WindowSlot::new(0),
            KeyCode::Enter,
        ));
        assert!(report.actions.is_empty());
        assert!(!core.exit_requested());
    }

    #[test]
    fn inject_pointer_button_routes_with_redraw() {
        let mut core = dual_core();
        let report = core.inject_event(SyntheticInput::pointer_button(
            WindowSlot::new(0),
            MouseButton::Left,
            true,
        ));
        assert_eq!(report.actions, vec![Action::Redraw(WindowSlot::new(0))]);
        assert_eq!(report.global_position, None);
    }

    #[test]
    fn inject_ime_sequence_is_plumbed() {
        let mut core = dual_core();
        let slot = WindowSlot::new(0);
        core.inject_event(SyntheticInput::ime(slot, Ime::Enabled));
        assert_eq!(*core.ime(), ImeStatus::Active);
        core.inject_event(SyntheticInput::ime(
            slot,
            Ime::Preedit("ni".to_owned(), None),
        ));
        assert_eq!(*core.ime(), ImeStatus::Preedit("ni".to_owned()));
        core.inject_event(SyntheticInput::ime(
            slot,
            Ime::Preedit("nih".to_owned(), Some((3, 3))),
        ));
        assert_eq!(*core.ime(), ImeStatus::Preedit("nih".to_owned()));
        core.inject_event(SyntheticInput::ime(slot, Ime::Commit("日".to_owned())));
        assert_eq!(*core.ime(), ImeStatus::Active);
        assert_eq!(core.last_commit(), Some("日"));
        core.inject_event(SyntheticInput::ime(slot, Ime::Disabled));
        assert_eq!(*core.ime(), ImeStatus::Inactive);
        // Commit survives disable (todo 22 consumes it).
        assert_eq!(core.last_commit(), Some("日"));
    }

    #[test]
    fn unknown_slot_injection_is_inert() {
        let mut core = dual_core();
        let report = core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(7),
            10.0,
            10.0,
        ));
        assert_eq!(report.global_position, None);
        assert_eq!(report.clamped_position, None);
        assert!(report.actions.is_empty());
        assert!(core.cursor().is_none());
        // Esc from an unknown slot still tears down (teardown must not depend
        // on slot bookkeeping).
        let report = core.inject_event(SyntheticInput::key_press(
            WindowSlot::new(7),
            KeyCode::Escape,
        ));
        assert_eq!(report.actions, vec![Action::Exit]);
    }

    #[test]
    fn non_finite_motion_leaves_cursor_state_untouched() {
        let mut core = dual_core();
        core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(0),
            10.0,
            20.0,
        ));
        let before = core.cursor().copied();
        let report = core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(0),
            f64::NAN,
            0.0,
        ));
        assert_eq!(report.global_position, None);
        assert!(report.actions.is_empty());
        assert_eq!(core.cursor().copied(), before);
    }
}
