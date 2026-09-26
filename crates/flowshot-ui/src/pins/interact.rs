//! The pin input dispatch: [`PinState`] event handlers (the child-module
//! impl pattern of todo 16 - private fields stay in [`super::state`], the
//! behavior lives here).
//!
//! Behavior map (F27 pin spec + plan todo 30):
//! - wheel: accumulate-then-commit zoom ANCHORED AT CURSOR (center-zoom
//!   AVOIDED);
//! - left press + move: `drag_window()` (the `startSystemMove` BORROW;
//!   deferred to first motion so the double-click-close parity survives the
//!   compositor drag grab, which swallows the second click);
//! - left double-click / Esc / Ctrl+Q / menu Close: close the pin;
//! - right release: toggle the context menu at the cursor;
//! - keys 0-9: absolute opacity 1.0..0.1 (F27 table: `0` -> 1.0, `d` ->
//!   d/10); R / Shift+R: rotate clockwise / counter-clockwise (the bindable
//!   rotate keys; todo 36 makes them configurable);
//! - touch: two-finger pinch preview + commit ([`super::pinch`]).

use std::time::{Duration, Instant};

use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use super::event::{PinEffect, PinInput};
use super::menu::{MenuAction, PinMenu};
use super::spec::{DOUBLE_CLICK_MS, OPACITY_TENTHS};
use super::state::PinState;
use super::zoom::commit_wheel;
use crate::render::{Point, f32_from_f64, f32_from_u32};

pub(super) fn dispatch(state: &mut PinState, input: &PinInput, now: Instant) -> Vec<PinEffect> {
    match *input {
        PinInput::CursorMoved { x, y } => state.cursor_moved(x, y),
        PinInput::CursorEntered => {
            state.hover = true;
            vec![PinEffect::Redraw]
        }
        PinInput::CursorLeft => {
            state.hover = false;
            vec![PinEffect::Redraw]
        }
        PinInput::Wheel { units } => {
            let steps = commit_wheel(&mut state.wheel_accum, units);
            state.zoom_by(steps)
        }
        PinInput::Button { button, pressed } => state.button(button, pressed, now),
        PinInput::Key {
            code,
            pressed: true,
            ..
        } => state.key(code),
        PinInput::Key { .. } => Vec::new(),
        PinInput::Modifiers(modifiers) => {
            state.modifiers = modifiers;
            Vec::new()
        }
        PinInput::Touch { id, phase, x, y } => state.touch(id, phase, (x, y)),
        PinInput::Resized { width, height } => {
            state.target_window = (width, height);
            vec![PinEffect::Redraw]
        }
        PinInput::ScaleFactorChanged { scale_factor } => state.rescale(scale_factor),
        PinInput::CloseRequested => vec![PinEffect::Close],
    }
}

impl PinState {
    fn cursor_moved(&mut self, x: f64, y: f64) -> Vec<PinEffect> {
        self.cursor = Some((x, y));
        let mut effects = Vec::new();
        if let Some(menu) = self.menu.as_mut() {
            let scale = f32_from_f64(self.scale_factor);
            let point = Point::new(f32_from_f64(x), f32_from_f64(y));
            if menu.hover(point, &self.behavior.tokens, scale) {
                effects.push(PinEffect::Redraw);
            }
        }
        if self.left_pressed && !self.drag_started {
            self.drag_started = true;
            effects.push(PinEffect::StartDrag);
        }
        effects
    }

    fn button(&mut self, button: MouseButton, pressed: bool, now: Instant) -> Vec<PinEffect> {
        match (button, pressed) {
            (MouseButton::Left, true) => self.left_pressed(now),
            (MouseButton::Left, false) => {
                self.left_pressed = false;
                Vec::new()
            }
            (MouseButton::Right, false) => self.toggle_menu(),
            _ => Vec::new(),
        }
    }

    fn left_pressed(&mut self, now: Instant) -> Vec<PinEffect> {
        if let Some(menu) = self.menu.take() {
            let mut effects = vec![PinEffect::Redraw];
            let point = self.cursor_point();
            let scale = f32_from_f64(self.scale_factor);
            if let Some(action) = menu.action_at(point, &self.behavior.tokens, scale) {
                effects.extend(self.menu_action(action));
            }
            return effects;
        }
        if let Some(last) = self.last_left_press
            && now.duration_since(last) <= Duration::from_millis(DOUBLE_CLICK_MS)
        {
            self.last_left_press = None;
            return vec![PinEffect::Close];
        }
        self.last_left_press = Some(now);
        self.left_pressed = true;
        self.drag_started = false;
        Vec::new()
    }

    fn toggle_menu(&mut self) -> Vec<PinEffect> {
        if self.menu.is_some() {
            self.menu = None;
            return vec![PinEffect::Redraw];
        }
        let point = self.cursor_point();
        let scale = f32_from_f64(self.scale_factor);
        let window = (
            f32_from_u32(self.target_window.0),
            f32_from_u32(self.target_window.1),
        );
        self.menu = Some(PinMenu::open(point, window, &self.behavior.tokens, scale));
        vec![PinEffect::Redraw]
    }

    fn menu_action(&mut self, action: MenuAction) -> Vec<PinEffect> {
        match action {
            MenuAction::Copy => vec![PinEffect::Copy],
            MenuAction::Save => vec![PinEffect::Save],
            MenuAction::RotateRight => self.rotate(true),
            MenuAction::RotateLeft => self.rotate(false),
            MenuAction::IncreaseOpacity => self.bump_opacity(1),
            MenuAction::DecreaseOpacity => self.bump_opacity(-1),
            MenuAction::Close => vec![PinEffect::Close],
        }
    }

    fn key(&mut self, code: KeyCode) -> Vec<PinEffect> {
        if code == KeyCode::Escape {
            if self.menu.is_some() {
                self.menu = None;
                return vec![PinEffect::Redraw];
            }
            return vec![PinEffect::Close];
        }
        if code == KeyCode::KeyQ && self.modifiers.control_key() {
            return vec![PinEffect::Close];
        }
        if code == KeyCode::KeyR {
            return self.rotate(!self.modifiers.shift_key());
        }
        match digit_tenths(code) {
            Some(tenths) => self.set_opacity(tenths),
            None => Vec::new(),
        }
    }

    /// The F27 opacity table: key `0` -> 1.0 (ten tenths), keys `1..=9` ->
    /// 0.1..0.9 absolute.
    fn set_opacity(&mut self, tenths: u8) -> Vec<PinEffect> {
        if self.opacity_tenths == tenths {
            return Vec::new();
        }
        self.opacity_tenths = tenths;
        vec![PinEffect::Reupload, PinEffect::Redraw]
    }

    fn bump_opacity(&mut self, delta: i16) -> Vec<PinEffect> {
        let tenths = i16::from(self.opacity_tenths) + delta;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into [0, OPACITY_TENTHS] before the cast"
        )]
        let clamped = tenths.clamp(0, i16::from(OPACITY_TENTHS)) as u8;
        self.set_opacity(clamped)
    }
}

/// Maps a digit key (top row or numpad) to its F27 opacity tenths:
/// `0` -> 10 (fully opaque), `1..=9` -> 1..=9.
fn digit_tenths(code: KeyCode) -> Option<u8> {
    let digit = match code {
        KeyCode::Digit0 | KeyCode::Numpad0 => 0,
        KeyCode::Digit1 | KeyCode::Numpad1 => 1,
        KeyCode::Digit2 | KeyCode::Numpad2 => 2,
        KeyCode::Digit3 | KeyCode::Numpad3 => 3,
        KeyCode::Digit4 | KeyCode::Numpad4 => 4,
        KeyCode::Digit5 | KeyCode::Numpad5 => 5,
        KeyCode::Digit6 | KeyCode::Numpad6 => 6,
        KeyCode::Digit7 | KeyCode::Numpad7 => 7,
        KeyCode::Digit8 | KeyCode::Numpad8 => 8,
        KeyCode::Digit9 | KeyCode::Numpad9 => 9,
        _ => return None,
    };
    Some(if digit == 0 { OPACITY_TENTHS } else { digit })
}
