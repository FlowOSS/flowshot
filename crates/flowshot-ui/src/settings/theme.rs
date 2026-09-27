//! Token-driven egui theming (plan todo 36: "`egui::Visuals` built FROM
//! design tokens").
//!
//! The settings window is the single egui surface (draft D8(b) exception),
//! and it must still look like `FlowShot`: every color, radius, and spacing
//! value projected into [`egui::Style`] comes from [`DesignTokens`] (todo 2)
//! and the `[ui]` config group - accent/contrast pickers in the Interface
//! tab re-theme the window LIVE (immediate-mode re-projection each frame),
//! which is the "live-apply where safe (tokens-driven)" contract.
//!
//! Dark/light: [`ThemeMode`] is the RESOLVED preference. The system
//! preference itself arrives from the binary layer (ashpd Settings portal -
//! the purity gate keeps portal crates out of flowshot-ui); `ThemeChoice::
//! System` defers to it.
//!
//! Fonts: the vendored Inter (OFL-1.1, todo 19 asset) replaces egui's
//! proportional default; the monospace family keeps egui's bundled face.

use egui::{
    Color32, FontData, FontDefinitions, FontFamily, Rounding, Style, TextStyle, Visuals,
    style::Spacing,
};
use flowshot_core::config::UiConfig;
use flowshot_core::tokens::DesignTokens;

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
const INTER_REGULAR: &[u8] = include_bytes!("../../fonts/Inter-Regular.ttf");

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

fn color_or(hex: &str, fallback: Color32) -> Color32 {
    parse_hex_rgb(hex).map_or(fallback, |[r, g, b]| Color32::from_rgb(r, g, b))
}

/// Picks readable ink for a filled `background` (WCAG relative luminance).
fn ink_on(background: Color32) -> Color32 {
    let [r, g, b, _] = background.to_array();
    let luminance = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    if luminance > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Font definitions with the vendored Inter as the proportional face
/// (the monospace family keeps egui's bundled face - Inter has no
/// monospaced metrics).
#[must_use]
pub fn fonts() -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    definitions
        .font_data
        .insert("Inter".to_owned(), FontData::from_static(INTER_REGULAR));
    definitions
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    definitions
}

/// Builds the egui style from design tokens + the `[ui]` config group.
///
/// Invalid hex values in `ui` fall back to the token palette (a corrupt
/// color must never panic or block the window - the model's validation
/// flags it for Apply instead).
#[must_use]
pub fn style(tokens: &DesignTokens, ui: &UiConfig, mode: ThemeMode) -> Style {
    let token_accent = color_or(&tokens.palette.accent, Color32::from_rgb(0x63, 0x66, 0xF1));
    let token_contrast = color_or(
        &tokens.palette.contrast,
        Color32::from_rgb(0x0F, 0x17, 0x2A),
    );
    let accent = color_or(&ui.accent_color, token_accent);
    let contrast = color_or(&ui.contrast_color, token_contrast);

    let mut visuals = match mode {
        ThemeMode::Dark => Visuals::dark(),
        ThemeMode::Light => Visuals::light(),
    };
    match mode {
        ThemeMode::Dark => {
            visuals.panel_fill = contrast;
            visuals.window_fill = contrast;
            visuals.extreme_bg_color = Color32::from_black_alpha(96);
        }
        ThemeMode::Light => {
            visuals.widgets.noninteractive.fg_stroke.color = contrast;
        }
    }
    visuals.selection.bg_fill = accent.linear_multiply(0.4);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, accent);
    visuals.hyperlink_color = accent;
    visuals.widgets.active.weak_bg_fill = accent;
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.fg_stroke.color = ink_on(accent);

    // Radii tokens: widgets are "buttons and cards" (medium), windows are
    // dialogs (large), menus are popovers (medium).
    let medium = f32::from(u8::try_from(tokens.radii.medium).unwrap_or(4));
    let large = f32::from(u8::try_from(tokens.radii.large).unwrap_or(8));
    let rounding = Rounding::same(medium);
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.open.rounding = rounding;
    visuals.window_rounding = Rounding::same(large);
    visuals.menu_rounding = Rounding::same(medium);

    let spacing = Spacing {
        item_spacing: egui::vec2(
            f32::from(u8::try_from(tokens.spacing.medium).unwrap_or(8)),
            f32::from(u8::try_from(tokens.spacing.small).unwrap_or(4)),
        ),
        button_padding: egui::vec2(
            f32::from(u8::try_from(tokens.spacing.medium).unwrap_or(8)),
            f32::from(u8::try_from(tokens.spacing.small).unwrap_or(4)),
        ),
        ..Spacing::default()
    };

    let mut style = Style {
        visuals,
        spacing,
        ..Style::default()
    };
    let base_size = f32::from(u8::try_from(tokens.typography.base_size).unwrap_or(14));
    style.text_styles = [
        (TextStyle::Body, egui::FontId::proportional(base_size)),
        (TextStyle::Button, egui::FontId::proportional(base_size)),
        (
            TextStyle::Heading,
            egui::FontId::proportional(base_size * 1.4),
        ),
        (
            TextStyle::Monospace,
            egui::FontId::monospace(base_size - 1.0),
        ),
        (
            TextStyle::Small,
            egui::FontId::proportional(base_size - 2.0),
        ),
    ]
    .into();
    style
}
