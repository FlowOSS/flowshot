//! Normalized input events and routing results.
//!
//! The winit shell translates [`WindowEvent`](winit::event::WindowEvent)s into
//! [`InputEvent`]s, dropping opaque platform handles (`WindowId`, `DeviceId`)
//! in favor of a [`WindowSlot`]. Because every payload type here is a plain
//! constructible value, the whole input path - routing, coordinate mapping,
//! teardown - is exercisable headlessly through the `test-drive` seam without
//! an event loop, a window, or a GPU.

use flowshot_core::geometry::LogicalPoint;
use winit::event::{Ime, MouseButton};
use winit::keyboard::{KeyCode, ModifiersState};

use crate::router::WindowSlot;

/// A window-normalized input event.
///
/// Pointer positions are surface-local in physical pixels (fractional, as
/// delivered by the compositor); the router converts them to global logical
/// space.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// The pointer moved within (or, during an implicit grab, beyond) the
    /// window surface.
    PointerMoved {
        /// Surface-local horizontal position in physical pixels.
        x: f64,
        /// Surface-local vertical position in physical pixels.
        y: f64,
    },

    /// A pointer button transitioned.
    PointerButton {
        /// The button that transitioned.
        button: MouseButton,
        /// `true` on press, `false` on release.
        pressed: bool,
    },

    /// A key transitioned (physical key code, layout-independent).
    Key {
        /// The physical key.
        code: KeyCode,
        /// `true` on press, `false` on release.
        pressed: bool,
        /// `true` when the press is an auto-repeat.
        repeat: bool,
    },

    /// The keyboard modifier snapshot changed (winit delivers modifiers as
    /// their own event; the selection engine reads the tracked state when
    /// routing keys and pointer events - plan todo 16).
    Modifiers(ModifiersState),

    /// An input-method event, plumbed through for the text tool (todo 22).
    Ime(Ime),
}

/// A synthetic input targeted at one window slot.
///
/// This is the payload of the `test-drive` seam
/// ([`OverlayCore::inject_event`](crate::OverlayCore::inject_event) and
/// `OverlayHandle::inject_event`), making mouse paths QA-able without
/// external injection tools.
#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticInput {
    /// The window slot the event is delivered to.
    pub slot: WindowSlot,
    /// The event to route.
    pub event: InputEvent,
}

impl SyntheticInput {
    /// A pointer-motion event at surface-local physical coordinates.
    #[must_use]
    pub const fn pointer_moved(slot: WindowSlot, x: f64, y: f64) -> Self {
        Self {
            slot,
            event: InputEvent::PointerMoved { x, y },
        }
    }

    /// A key-press event.
    #[must_use]
    pub const fn key_press(slot: WindowSlot, code: KeyCode) -> Self {
        Self {
            slot,
            event: InputEvent::Key {
                code,
                pressed: true,
                repeat: false,
            },
        }
    }

    /// A key-release event.
    #[must_use]
    pub const fn key_release(slot: WindowSlot, code: KeyCode) -> Self {
        Self {
            slot,
            event: InputEvent::Key {
                code,
                pressed: false,
                repeat: false,
            },
        }
    }

    /// A pointer button press-or-release event.
    #[must_use]
    pub const fn pointer_button(slot: WindowSlot, button: MouseButton, pressed: bool) -> Self {
        Self {
            slot,
            event: InputEvent::PointerButton { button, pressed },
        }
    }

    /// A modifier-state change event.
    #[must_use]
    pub const fn modifiers(slot: WindowSlot, modifiers: ModifiersState) -> Self {
        Self {
            slot,
            event: InputEvent::Modifiers(modifiers),
        }
    }

    /// An input-method event.
    #[must_use]
    pub const fn ime(slot: WindowSlot, ime: Ime) -> Self {
        Self {
            slot,
            event: InputEvent::Ime(ime),
        }
    }
}

/// An effect the winit shell must apply after routing an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Request a redraw of the given window.
    Redraw(WindowSlot),
    /// Tear down every window and exit the event loop.
    Exit,
    /// The selection was accepted (Enter): the binary layer runs the export
    /// actions (todo 35 wires accept -> export -> teardown).
    Accept,
    /// The selection was copied (Ctrl+C or a configured double-click): the
    /// binary layer runs the clipboard pipeline (todos 28/35).
    Copy,
    /// Right-click: open the color wheel at the shared cursor position
    /// (todo 26 seam).
    ColorWheel,
}

/// The outcome of routing one input event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RouteReport {
    /// The mapped position in global logical space, unclamped: during an
    /// implicit pointer grab, positions beyond the window surface extend
    /// across the layout (the cross-monitor spanning enabler). `None` when
    /// the event carried no position or the slot is unknown.
    pub global_position: Option<LogicalPoint>,
    /// [`Self::global_position`] clamped to the layout's union bounds.
    pub clamped_position: Option<LogicalPoint>,
    /// Effects for the shell to apply.
    pub actions: Vec<Action>,
}

/// The plumbed IME session state (always-on model, draft D7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ImeStatus {
    /// No IME session is active.
    #[default]
    Inactive,
    /// The IME is enabled and awaiting composition.
    Active,
    /// Composition is in progress with the given preedit text.
    Preedit(String),
}
