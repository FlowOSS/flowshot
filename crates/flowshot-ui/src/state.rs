//! Headless overlay state and the central routing entry point.
//!
//! [`OverlayCore`] owns the [`InputRouter`] plus the input-derived state every
//! window shares (cursor track, IME status, exit request). It contains no
//! windowing or GPU handles, so the full input path - coordinate mapping,
//! Esc teardown, IME plumbing - is unit-testable headlessly through the
//! `test-drive` seam [`OverlayCore::inject_event`].

use std::time::Instant;

use flowshot_core::geometry::LogicalPoint;
use winit::event::Ime;
use winit::keyboard::KeyCode;

#[cfg(any(test, feature = "test-drive"))]
use crate::input::SyntheticInput;
use crate::input::{Action, ImeStatus, InputEvent, RouteReport};
use crate::router::{InputRouter, WindowSlot};

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
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayCore {
    router: InputRouter,
    cursor: Option<CursorTrack>,
    ime: ImeStatus,
    last_commit: Option<String>,
    exit_requested: bool,
}

impl OverlayCore {
    /// Creates a core around `router`.
    #[must_use]
    pub const fn new(router: InputRouter) -> Self {
        Self {
            router,
            cursor: None,
            ime: ImeStatus::Inactive,
            last_commit: None,
            exit_requested: false,
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

    /// Routes one normalized event: maps coordinates into global logical
    /// space, updates shared state, and returns the effects for the shell.
    ///
    /// This is the single funnel every input source passes through - real
    /// winit events and synthetic test-drive injections alike. Crate-internal:
    /// external injection goes through the feature-gated
    /// [`Self::inject_event`] seam.
    pub(crate) fn route(&mut self, slot: WindowSlot, event: &InputEvent) -> RouteReport {
        match event {
            InputEvent::PointerMoved { x, y } => self.route_motion(slot, *x, *y),
            InputEvent::PointerButton { button, pressed } => {
                tracing::trace!(window = slot.index(), ?button, pressed, "pointer button");
                RouteReport {
                    actions: vec![Action::Redraw(slot)],
                    ..RouteReport::default()
                }
            }
            InputEvent::Key {
                code,
                pressed,
                repeat,
            } => self.route_key(slot, *code, *pressed, *repeat),
            InputEvent::Ime(ime) => {
                self.apply_ime(ime);
                RouteReport::default()
            }
        }
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
        let actions = global.map_or_else(Vec::new, |_| vec![Action::Redraw(slot)]);
        RouteReport {
            global_position: global,
            clamped_position: clamped,
            actions,
        }
    }

    fn route_key(
        &mut self,
        slot: WindowSlot,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
    ) -> RouteReport {
        // Latency span + elapsed sample feed the todo-38 keypress->map budget.
        let started = Instant::now();
        let _span =
            tracing::trace_span!("input.key_to_map", window = slot.index(), pressed, repeat)
                .entered();
        let mut actions = Vec::new();
        if pressed && code == KeyCode::Escape {
            self.exit_requested = true;
            actions.push(Action::Exit);
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
    use flowshot_core::geometry::{LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform};
    use winit::event::MouseButton;

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
