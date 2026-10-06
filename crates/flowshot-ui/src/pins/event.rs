//! Pin input/effect vocabulary: the shell translates winit events into
//! [`PinInput`], the pure [`super::state::PinState`] answers with
//! [`PinEffect`]s, and the shell applies them. Every payload is a plain
//! constructible value, so the whole pin behavior spec is exercisable
//! headlessly through the `test-drive` seam (the inject-seam pattern).

use winit::event::{MouseButton, TouchPhase};
use winit::keyboard::{KeyCode, ModifiersState};

/// A pin-window input event (window-local physical px where positioned).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PinInput {
    /// The pointer moved inside the window.
    CursorMoved {
        /// Window-local horizontal position, physical px.
        x: f64,
        /// Window-local vertical position, physical px.
        y: f64,
    },
    /// The pointer entered the window (shadow hover highlight).
    CursorEntered,
    /// The pointer left the window.
    CursorLeft,
    /// Accumulated scroll in wheel units (one discrete notch = 120; the
    /// shell converts `LineDelta`/`PixelDelta` - see [`super::spec`]).
    Wheel {
        /// Signed scroll amount in wheel units (up = positive = zoom in).
        units: f64,
    },
    /// A pointer button transitioned.
    Button {
        /// The button.
        button: MouseButton,
        /// `true` on press.
        pressed: bool,
    },
    /// A key transitioned (physical code, layout-independent).
    Key {
        /// The physical key.
        code: KeyCode,
        /// `true` on press.
        pressed: bool,
        /// `true` on auto-repeat.
        repeat: bool,
    },
    /// The keyboard modifier snapshot changed (Ctrl+Q close, Shift+R
    /// rotate counter-clockwise).
    Modifiers(ModifiersState),
    /// A touch point transitioned (pinch zoom source).
    Touch {
        /// Compositor touch-point id.
        id: u64,
        /// Touch phase.
        phase: TouchPhase,
        /// Window-local horizontal position, physical px.
        x: f64,
        /// Window-local vertical position, physical px.
        y: f64,
    },
    /// The compositor applied a new window extent (ground truth after a
    /// resize request; the state adopts it and repaints).
    Resized {
        /// New width, physical px.
        width: u32,
        /// New height, physical px.
        height: u32,
    },
    /// The window moved to a monitor with a different scale factor.
    ScaleFactorChanged {
        /// The new device scale factor.
        scale_factor: f64,
    },
    /// The compositor or user requested the window closed.
    CloseRequested,
}

/// An effect the pin shell must apply after routing an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinEffect {
    /// Repaint the window.
    Redraw,
    /// Resize the window to exactly this physical extent (the shell pins
    /// min+max inner size - the only client resize path Hyprland honors -
    /// and issues the X11 `ConfigureRequest`; see [`super::zoom`] module
    /// header).
    SetWindowSize {
        /// Target width, physical px.
        width: u32,
        /// Target height, physical px.
        height: u32,
    },
    /// Re-derive and re-upload the image texture (rotation or opacity
    /// changed the pixel buffer).
    Reupload,
    /// Begin a compositor-driven window drag (`drag_window()`, the
    /// `startSystemMove` equivalent - F27 BORROW).
    StartDrag,
    /// Close this pin window.
    Close,
    /// Hand the current snapshot to the action sink (the clipboard module).
    Copy,
    /// Hand the current snapshot to the action sink (the export module).
    Save,
}
