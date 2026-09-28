//! The derived surface scale: window / card / field / ink / separator colors
//! for both theme modes.
//!
//! The palette tokens carry exactly two colors (accent + contrast), so the
//! full chrome is DERIVED: every surface here is a palette token mixed
//! toward black/white through a named ratio. The hue always follows the
//! contrast token - change `[ui].contrast_color` and the whole scale
//! re-derives live (the tokens-driven live-apply contract). The ratios are
//! the design system's surface recipe, not free-standing color constants.

use egui::Color32;

use super::ThemeMode;

/// Dark mode: how far the card surface lifts from the window (contrast
/// token) toward white.
const CARD_LIFT_DARK: f32 = 0.06;
/// Dark mode: how far a text field sinks from the card toward black.
const FIELD_SINK_DARK: f32 = 0.30;
/// Dark mode: resting button surface lift from the card toward the ink.
const BUTTON_LIFT_DARK: f32 = 0.08;
/// Dark mode: hovered button surface lift from the card toward the ink.
const BUTTON_HOVER_DARK: f32 = 0.16;
/// Dark mode: control surfaces (slider rail, checkbox box at rest, scroll
/// handle) lift from the card toward the ink - brighter than the field
/// wells so handles and rails read against them.
const CONTROL_LIFT_DARK: f32 = 0.20;
/// Dark mode: hovered control surface lift.
const CONTROL_HOVER_DARK: f32 = 0.28;
/// Dark mode: popup surface lift from the card toward white.
const POPUP_LIFT_DARK: f32 = 0.04;
/// Light mode: how far the window surface tints from white toward the
/// contrast token (cards stay pure white on top - the GNOME pattern).
const WINDOW_TINT_LIGHT: f32 = 0.06;
/// Light mode: how far a text field tints from the card toward the contrast.
const FIELD_TINT_LIGHT: f32 = 0.045;
/// Light mode: resting button surface tint from the card toward the
/// contrast.
const BUTTON_TINT_LIGHT: f32 = 0.06;
/// Light mode: hovered button surface tint from the card toward the
/// contrast.
const BUTTON_HOVER_TINT_LIGHT: f32 = 0.12;
/// Light mode: control surfaces (slider rail, checkbox box, scroll handle)
/// tint from the card toward the contrast.
const CONTROL_TINT_LIGHT: f32 = 0.16;
/// Light mode: hovered control surface tint.
const CONTROL_HOVER_TINT_LIGHT: f32 = 0.24;
/// Light mode: how far the body ink softens from the contrast token toward
/// black (the contrast token IS the light-mode ink; never pure black).
const INK_SOFTEN_LIGHT: f32 = 0.25;
/// Body ink softening in dark mode: how far the ink pulls from pure white
/// toward the window surface (never glare-white on dark).
const INK_SOFTEN_DARK: f32 = 0.10;
/// Weak (secondary) text: gamma multiplier over the body ink.
const WEAK_TEXT: f32 = 0.62;
/// Separator lines: alpha multiplier over the ink.
const SEPARATOR_ALPHA: f32 = 0.12;
/// Control outlines: alpha multiplier over the ink.
const OUTLINE_ALPHA: f32 = 0.24;
/// Strong outlines (color-swatch frames, popup edges): alpha multiplier
/// over the ink - the resting outline is too faint to frame a near-black
/// swatch on a dark card.
const OUTLINE_STRONG_ALPHA: f32 = 0.45;

/// The derived surface scale for one (contrast, mode) pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surfaces {
    /// The window background (egui `panel_fill`).
    pub window: Color32,
    /// The raised section-card background.
    pub card: Color32,
    /// The sunken text-field / slider-rail background.
    pub field: Color32,
    /// The popup (combo menu) background.
    pub popup: Color32,
    /// The resting button surface.
    pub button: Color32,
    /// The hovered button surface.
    pub button_hover: Color32,
    /// Control surfaces at rest: slider rail, checkbox box, scroll handle.
    pub control: Color32,
    /// Control surfaces hovered.
    pub control_hover: Color32,
    /// Hairline separators (title rules, section dividers).
    pub separator: Color32,
    /// Control outlines (field/button borders at rest).
    pub outline: Color32,
    /// Strong outlines (color-swatch frames, popup edges).
    pub outline_strong: Color32,
    /// Body text ink.
    pub text: Color32,
    /// Secondary/hint text ink.
    pub text_weak: Color32,
}

/// Per-channel byte mix (both endpoints are opaque sRGB bytes; the result
/// stays in the same byte space the readback asserts measure).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the mixed channel is clamped to 0..=255 before the byte cast"
)]
fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let [fr, fg, fb, _] = from.to_array();
    let [tr, tg, tb, _] = to.to_array();
    let channel = |a: u8, b: u8| {
        let mixed = f32::from(a) + (f32::from(b) - f32::from(a)) * t;
        mixed.round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(channel(fr, tr), channel(fg, tg), channel(fb, tb))
}

/// Derives the full surface scale from the resolved contrast token.
#[must_use]
pub fn surfaces(contrast: Color32, mode: ThemeMode) -> Surfaces {
    match mode {
        ThemeMode::Dark => {
            let window = contrast;
            let card = mix(window, Color32::WHITE, CARD_LIFT_DARK);
            let ink = mix(Color32::WHITE, window, INK_SOFTEN_DARK);
            Surfaces {
                window,
                card,
                field: mix(card, Color32::BLACK, FIELD_SINK_DARK),
                popup: mix(card, Color32::WHITE, POPUP_LIFT_DARK),
                button: mix(card, ink, BUTTON_LIFT_DARK),
                button_hover: mix(card, ink, BUTTON_HOVER_DARK),
                control: mix(card, ink, CONTROL_LIFT_DARK),
                control_hover: mix(card, ink, CONTROL_HOVER_DARK),
                separator: ink.gamma_multiply(SEPARATOR_ALPHA),
                outline: ink.gamma_multiply(OUTLINE_ALPHA),
                outline_strong: ink.gamma_multiply(OUTLINE_STRONG_ALPHA),
                text: ink,
                text_weak: ink.gamma_multiply(WEAK_TEXT),
            }
        }
        ThemeMode::Light => {
            let window = mix(Color32::WHITE, contrast, WINDOW_TINT_LIGHT);
            let card = Color32::WHITE;
            let ink = mix(contrast, Color32::BLACK, INK_SOFTEN_LIGHT);
            Surfaces {
                window,
                card,
                field: mix(card, contrast, FIELD_TINT_LIGHT),
                popup: card,
                button: mix(card, contrast, BUTTON_TINT_LIGHT),
                button_hover: mix(card, contrast, BUTTON_HOVER_TINT_LIGHT),
                control: mix(card, contrast, CONTROL_TINT_LIGHT),
                control_hover: mix(card, contrast, CONTROL_HOVER_TINT_LIGHT),
                separator: contrast.gamma_multiply(SEPARATOR_ALPHA),
                outline: contrast.gamma_multiply(OUTLINE_ALPHA),
                outline_strong: contrast.gamma_multiply(OUTLINE_STRONG_ALPHA),
                text: ink,
                text_weak: ink.gamma_multiply(WEAK_TEXT),
            }
        }
    }
}
