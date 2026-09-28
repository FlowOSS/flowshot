//! Token-driven egui theming ("`egui::Visuals` built FROM
//! design tokens").
//!
//! The settings window and the capture launcher dialog
//! are the only egui surfaces (draft D8(b) exception), and both must still
//! look like `FlowShot`: every color, radius, and spacing value projected
//! into [`egui::Style`] comes from [`DesignTokens`] and the `[ui]`
//! config group - accent/contrast pickers in the Interface tab re-theme the
//! window LIVE (immediate-mode re-projection each frame), which is the
//! "live-apply where safe (tokens-driven)" contract.
//!
//! # Layers
//!
//! - [`surfaces`]: the derived surface scale (window / card / field / ink /
//!   separator) - every color is a palette token mixed through a named
//!   ratio, so the whole chrome follows the contrast token.
//! - [`style`]: the shared token projection (colors, radii, typography) both
//!   egui windows use.
//! - [`settings_style`]: `style` plus the form-grid geometry (uniform
//!   control height, icon edge, slider/combo widths) that BOTH egui panels
//!   project - one control vocabulary across the settings window and the
//!   launcher dialog (each keeps its own window size).
//!
//! Dark/light: [`ThemeMode`] is the RESOLVED preference. The system
//! preference itself arrives from the binary layer (ashpd Settings portal -
//! the purity gate keeps portal crates out of flowshot-ui); `ThemeChoice::
//! System` defers to it.
//!
//! Fonts: the vendored Inter family (OFL-1.1 - regular,
//! medium, and semibold weights) replaces egui's proportional default; the
//! monospace family keeps egui's bundled face. The medium/semibold weights
//! back the typographic hierarchy (section titles, active tab, primary
//! button).

mod projection;
mod surfaces;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};
use flowshot_core::config::UiConfig;
use flowshot_core::tokens::DesignTokens;

pub use projection::{control_height, settings_style, style};
pub use surfaces::{Surfaces, surfaces};

/// The resolved dark/light preference driving [`style`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Dark surfaces, light text.
    #[default]
    Dark,
    /// Light surfaces, dark text.
    Light,
}

/// The vendored Inter regular face (OFL-1.1; `fonts/LICENSE.txt`).
const INTER_REGULAR: &[u8] = include_bytes!("../../../fonts/Inter-Regular.ttf");
/// The vendored Inter medium face (sub-section titles, resting tab labels).
const INTER_MEDIUM: &[u8] = include_bytes!("../../../fonts/Inter-Medium.ttf");
/// The vendored Inter semibold face (section titles, active tab, Apply).
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../fonts/Inter-SemiBold.ttf");

/// The font-data key + named family of the medium weight.
pub const MEDIUM_FAMILY: &str = "Inter Medium";
/// The font-data key + named family of the semibold weight.
pub const SEMIBOLD_FAMILY: &str = "Inter SemiBold";

/// Parses a strict `#RRGGBB` color (the config's documented format).
#[must_use]
pub fn parse_hex_rgb(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_char_boundary(6) {
        return None;
    }
    let channel = |offset: usize| u8::from_str_radix(digits.get(offset..offset + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Parses a strict `#RRGGBBAA` color (the shadow tokens' format).
#[must_use]
pub fn parse_hex_rgba(hex: &str) -> Option<[u8; 4]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 8 || !digits.is_char_boundary(8) {
        return None;
    }
    let channel = |offset: usize| u8::from_str_radix(digits.get(offset..offset + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?, channel(6)?])
}

/// The config-over-token color resolution shared by [`style`] and
/// [`surfaces_for`]: an invalid config hex falls back to the palette token
/// (a corrupt color must never panic or block the window - the model's
/// validation flags it for Apply instead).
pub(crate) fn resolve_colors(tokens: &DesignTokens, ui: &UiConfig) -> (Color32, Color32) {
    let token_accent = color_or(&tokens.palette.accent, Color32::from_rgb(0x63, 0x66, 0xF1));
    let token_contrast = color_or(
        &tokens.palette.contrast,
        Color32::from_rgb(0x0F, 0x17, 0x2A),
    );
    (
        color_or(&ui.accent_color, token_accent),
        color_or(&ui.contrast_color, token_contrast),
    )
}

pub(crate) fn color_or(hex: &str, fallback: Color32) -> Color32 {
    parse_hex_rgb(hex).map_or(fallback, |[r, g, b]| Color32::from_rgb(r, g, b))
}

/// The surface scale for a tokens + `[ui]` config pair - the QA entry the
/// offscreen pixel asserts derive their expected colors from (the same
/// resolution [`style`] projects).
#[must_use]
pub fn surfaces_for(tokens: &DesignTokens, ui: &UiConfig, mode: ThemeMode) -> Surfaces {
    let (_, contrast) = resolve_colors(tokens, ui);
    surfaces(contrast, mode)
}

/// Picks readable ink for a filled `background` (WCAG relative luminance).
#[must_use]
pub(crate) fn ink_on(background: Color32) -> Color32 {
    let [r, g, b, _] = background.to_array();
    let luminance = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    if luminance > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Font definitions with the vendored Inter weights: Regular backs the
/// proportional family, Medium/SemiBold are registered as named families
/// (with a Regular fallback chain) for the hierarchy helpers below. The
/// monospace family keeps egui's bundled face - Inter has no monospaced
/// metrics.
#[must_use]
pub fn fonts() -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    definitions
        .font_data
        .insert("Inter".to_owned(), FontData::from_static(INTER_REGULAR));
    definitions.font_data.insert(
        MEDIUM_FAMILY.to_owned(),
        FontData::from_static(INTER_MEDIUM),
    );
    definitions.font_data.insert(
        SEMIBOLD_FAMILY.to_owned(),
        FontData::from_static(INTER_SEMIBOLD),
    );
    definitions
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    definitions.families.insert(
        FontFamily::Name(MEDIUM_FAMILY.into()),
        vec![MEDIUM_FAMILY.to_owned(), "Inter".to_owned()],
    );
    definitions.families.insert(
        FontFamily::Name(SEMIBOLD_FAMILY.into()),
        vec![SEMIBOLD_FAMILY.to_owned(), "Inter".to_owned()],
    );
    definitions
}

/// The medium-weight face at `size` (sub-section titles, resting tabs).
#[must_use]
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// The semibold-weight face at `size` (section titles, active tab, the
/// primary button).
#[must_use]
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}
