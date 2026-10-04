//! The `winit` 0.30 -> `egui` input bridge (the egui-winit replacement).
//!
//! Accumulates window events between frames and drains them into an
//! [`egui::RawInput`], mirroring egui-winit 0.36 semantics: logical-key-first
//! keyboard mapping ([`super::keymap`]), line/pixel wheel units, printable
//! text filtering, Ctrl/Cmd clipboard command translation through the
//! optional [`ClipboardBridge`], and always-on IME forwarding (draft D7).
//!
//! Pointer positions are converted to POINTS (physical / `pixels_per_point`)
//! once, here - the single logical<->physical conversion point of this
//! surface (the #4871-family rule the whole crate follows).

use egui::{Event, ImeEvent, Modifiers, MouseWheelUnit, Pos2, Rect, TouchPhase, Vec2};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};

use super::keymap;

/// Clipboard access for Ctrl+C/V in egui text fields. The lib stays pure:
/// the binary layer wires the Wayland clipboard; `None` bridge =
/// clipboard shortcuts inert (keys still reach egui as plain events).
#[derive(Clone)]
pub struct ClipboardBridge {
    /// Reads the current clipboard text.
    pub get: std::sync::Arc<dyn Fn() -> Option<String> + Send + Sync + 'static>,
    /// Writes text to the clipboard.
    pub set: std::sync::Arc<dyn Fn(&str) + Send + Sync + 'static>,
}

impl ClipboardBridge {
    /// Wraps a get/set pair.
    pub fn new(
        get: impl Fn() -> Option<String> + Send + Sync + 'static,
        set: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            get: std::sync::Arc::new(get),
            set: std::sync::Arc::new(set),
        }
    }
}

impl std::fmt::Debug for ClipboardBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardBridge(..)")
    }
}

/// What a window event asks the window layer to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowSignal {
    /// Nothing; the event was consumed into the frame input.
    None,
    /// The pixel extent or scale factor changed (reconfigure the surface).
    Resized,
    /// The user asked to close the window.
    Close,
}

/// Per-window egui input accumulator.
#[derive(Debug)]
pub(crate) struct InputState {
    events: Vec<Event>,
    modifiers: Modifiers,
    pointer_pos: Option<Pos2>,
    screen_size_points: Vec2,
    pixels_per_point: f32,
    max_texture_side: u32,
    focused: bool,
    clipboard: Option<ClipboardBridge>,
    start: std::time::Instant,
}

impl InputState {
    pub(crate) fn new(
        pixels_per_point: f32,
        screen_size_points: Vec2,
        max_texture_side: u32,
        clipboard: Option<ClipboardBridge>,
    ) -> Self {
        Self {
            events: Vec::new(),
            modifiers: Modifiers::default(),
            pointer_pos: None,
            screen_size_points,
            pixels_per_point,
            max_texture_side,
            focused: true,
            clipboard,
            start: std::time::Instant::now(),
        }
    }

    pub(crate) fn set_screen_size_points(&mut self, size: Vec2) {
        self.screen_size_points = size;
    }

    pub(crate) fn set_pixels_per_point(&mut self, pixels_per_point: f32) {
        self.pixels_per_point = pixels_per_point;
    }

    #[cfg(feature = "test-drive")]
    pub(crate) const fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// TEST-DRIVE seam: queues one egui event directly (the pins'
    /// domain-level injection pattern - winit's `KeyEvent` has private
    /// fields, so synthetic keyboard input enters one layer below the
    /// winit conversion, which the keymap tests cover independently).
    #[cfg(feature = "test-drive")]
    pub(crate) fn push_event(&mut self, event: Event) {
        self.events.push(event);
    }

    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "f64 window coordinates/scale -> f32 egui points; UI extents are far within f32 range"
    )]
    pub(crate) fn on_window_event(&mut self, event: &WindowEvent) -> WindowSignal {
        match event {
            WindowEvent::Resized(size) => {
                self.screen_size_points = Vec2::new(
                    size.width as f32 / self.pixels_per_point,
                    size.height as f32 / self.pixels_per_point,
                );
                WindowSignal::Resized
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let factor = *scale_factor as f32;
                let scale_ratio = factor / self.pixels_per_point;
                self.pixels_per_point = factor;
                self.screen_size_points *= scale_ratio;
                WindowSignal::Resized
            }
            WindowEvent::CloseRequested => WindowSignal::Close,
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                WindowSignal::None
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = Pos2::new(
                    position.x as f32 / self.pixels_per_point,
                    position.y as f32 / self.pixels_per_point,
                );
                self.pointer_pos = Some(pos);
                self.events.push(Event::PointerMoved(pos));
                WindowSignal::None
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer_pos = None;
                self.events.push(Event::PointerGone);
                WindowSignal::None
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(pos) = self.pointer_pos {
                    self.events.push(Event::PointerButton {
                        pos,
                        button: pointer_button(*button),
                        pressed: *state == ElementState::Pressed,
                        modifiers: self.modifiers,
                    });
                }
                WindowSignal::None
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (unit, delta) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (MouseWheelUnit::Line, Vec2::new(*x, *y)),
                    MouseScrollDelta::PixelDelta(position) => (
                        MouseWheelUnit::Point,
                        Vec2::new(
                            position.x as f32 / self.pixels_per_point,
                            position.y as f32 / self.pixels_per_point,
                        ),
                    ),
                };
                self.events.push(Event::MouseWheel {
                    unit,
                    delta,
                    phase: touch_phase(*phase),
                    modifiers: self.modifiers,
                });
                WindowSignal::None
            }
            WindowEvent::ModifiersChanged(state) => {
                let state = state.state();
                self.modifiers = Modifiers {
                    alt: state.alt_key(),
                    ctrl: state.control_key(),
                    shift: state.shift_key(),
                    mac_cmd: false,
                    command: state.control_key(),
                };
                // egui 0.36 removed `RawInput::modifiers`; the held-modifier
                // set now travels as an event (the egui-winit 0.36 pattern).
                self.events.push(Event::ModifiersChanged(self.modifiers));
                WindowSignal::None
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.on_key_event(event);
                WindowSignal::None
            }
            WindowEvent::Ime(ime) => {
                match ime {
                    // egui 0.36 deprecated the Enabled/Disabled notifications
                    // ("no longer used by egui"); egui-winit drops them too.
                    Ime::Enabled | Ime::Disabled => {}
                    Ime::Preedit(text, active_range_bytes) => {
                        self.events.push(Event::Ime(ImeEvent::Preedit {
                            text: text.clone(),
                            active_range_chars: preedit_range_chars(text, *active_range_bytes),
                        }));
                    }
                    Ime::Commit(text) => {
                        self.events.push(Event::Ime(ImeEvent::Commit(text.clone())));
                    }
                }
                WindowSignal::None
            }
            _ => WindowSignal::None,
        }
    }

    fn on_key_event(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        let mapped = keymap::egui_key_for_event(&event.logical_key, event.physical_key);
        if let Some((key, physical_key)) = mapped {
            if pressed && self.on_clipboard_command(key) {
                return;
            }
            self.events.push(Event::Key {
                key,
                physical_key,
                pressed,
                repeat: event.repeat,
                modifiers: self.modifiers,
            });
        }
        if pressed
            && !self.modifiers.ctrl
            && !self.modifiers.command
            && let Some(text) = &event.text
            && !text.is_empty()
            && text.chars().all(is_printable_char)
        {
            self.events.push(Event::Text(text.to_string()));
        }
    }

    /// Translates Ctrl+C/X/V into the clipboard events egui expects;
    /// returns true when the key press was consumed as a command.
    fn on_clipboard_command(&mut self, key: egui::Key) -> bool {
        if !self.modifiers.command {
            return false;
        }
        match key {
            egui::Key::C => {
                self.events.push(Event::Copy);
                true
            }
            egui::Key::X => {
                self.events.push(Event::Cut);
                true
            }
            egui::Key::V => {
                if let Some(bridge) = &self.clipboard
                    && let Some(contents) = (bridge.get)()
                {
                    let contents = contents.replace("\r\n", "\n");
                    if !contents.is_empty() {
                        self.events.push(Event::Paste(contents));
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Writes egui's copy/cut output through the bridge, when present.
    pub(crate) fn push_copied_text(&self, text: &str) {
        if !text.is_empty()
            && let Some(bridge) = &self.clipboard
        {
            (bridge.set)(text);
        }
    }

    /// Drains the accumulated events into the frame's [`egui::RawInput`].
    pub(crate) fn take_raw_input(&mut self) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen_size_points)),
            max_texture_side: Some(usize::try_from(self.max_texture_side).unwrap_or(2048)),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: self.focused,
            ..Default::default()
        }
    }
}

fn touch_phase(phase: winit::event::TouchPhase) -> TouchPhase {
    match phase {
        winit::event::TouchPhase::Started => TouchPhase::Start,
        winit::event::TouchPhase::Moved => TouchPhase::Move,
        winit::event::TouchPhase::Ended => TouchPhase::End,
        winit::event::TouchPhase::Cancelled => TouchPhase::Cancel,
    }
}

/// winit reports the preedit cursor span in BYTES; egui wants CHAR ranges.
/// An out-of-bounds span (compositor bug) drops the range, never the text.
fn preedit_range_chars(
    text: &str,
    range_bytes: Option<(usize, usize)>,
) -> Option<std::ops::Range<usize>> {
    let (start_bytes, end_bytes) = range_bytes?;
    let start_chars = text.get(..start_bytes)?.chars().count();
    let middle_chars = text.get(start_bytes..end_bytes)?.chars().count();
    Some(start_chars..start_chars + middle_chars)
}

fn pointer_button(button: MouseButton) -> egui::PointerButton {
    match button {
        MouseButton::Right => egui::PointerButton::Secondary,
        MouseButton::Middle => egui::PointerButton::Middle,
        MouseButton::Back => egui::PointerButton::Extra1,
        MouseButton::Forward => egui::PointerButton::Extra2,
        MouseButton::Left | MouseButton::Other(_) => egui::PointerButton::Primary,
    }
}

/// egui-winit's printable-char filter: no ASCII controls, no private-use
/// areas (macOS sends delete as U+F728).
fn is_printable_char(chr: char) -> bool {
    let private_use = ('\u{e000}'..='\u{f8ff}').contains(&chr)
        || ('\u{f0000}'..='\u{ffffd}').contains(&chr)
        || ('\u{100000}'..='\u{10fffd}').contains(&chr);
    !private_use && !chr.is_ascii_control()
}
