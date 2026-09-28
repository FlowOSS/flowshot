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
        /// The text the key produced (winit `KeyEvent.text`): `None` on
        /// releases, non-text keys, and presses an IME consumed. The text tool's
        /// edit sessions insert this payload; with text-input-v3 active the
        /// compositor/IME keyboard grab guarantees text arrives through
        /// EITHER this field OR `Ime::Commit`, never both (the iced model).
        text: Option<String>,
    },

    /// The mouse wheel rotated, in angle-delta units (a standard notch is
    /// ±120; the shell converts winit's line/pixel deltas at
    /// [`WHEEL_ANGLE_PER_LINE`](crate::editor::WHEEL_ANGLE_PER_LINE) - the
    /// editor tool-size adjuster consumes this).
    Wheel {
        /// Vertical angle delta (positive = away from the user).
        delta_y: i32,
    },

    /// The keyboard modifier snapshot changed (winit delivers modifiers as
    /// their own event; the selection engine reads the tracked state when
    /// routing keys and pointer events).
    Modifiers(ModifiersState),

    /// An input-method event, plumbed through for the text tool.
    Ime(Ime),
}

/// A synthetic input targeted at one window slot.
///
/// This is the payload of the `test-drive` seam
/// (`OverlayCore::inject_event` and `OverlayHandle::inject_event`), making mouse paths QA-able without
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
                text: None,
            },
        }
    }

    /// A key-press event carrying produced text (the text tool's input
    /// seam: edit sessions insert the payload).
    #[must_use]
    pub fn key_text(slot: WindowSlot, code: KeyCode, text: &str) -> Self {
        Self {
            slot,
            event: InputEvent::Key {
                code,
                pressed: true,
                repeat: false,
                text: Some(text.to_owned()),
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
                text: None,
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

    /// A mouse-wheel event in angle-delta units (±120 = one standard notch).
    #[must_use]
    pub const fn wheel(slot: WindowSlot, delta_y: i32) -> Self {
        Self {
            slot,
            event: InputEvent::Wheel { delta_y },
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
    /// actions (accept -> export -> teardown).
    Accept,
    /// The selection was copied (Ctrl+C or a configured double-click): the
    /// binary layer runs the clipboard pipeline.
    Copy,
    /// Toolbar save: the binary layer saves the export to disk.
    Save,
    /// Toolbar pin: the binary layer pins the export to the screen.
    Pin,
    /// Toolbar upload: the binary layer uploads the export.
    Upload,
    /// Toolbar open-app: the binary layer saves + opens the export with
    /// another application.
    OpenWith,
    /// Right-click: open the color wheel at the shared cursor position
    /// (a chrome seam).
    ColorWheel,
    /// The eyedropper sampled a color (the funnel already delivered it to
    /// the color-pick sink; the shell arm is a no-op).
    ColorPicked,
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
