//! The `egui::Style` projection: [`style`] (shared colors/radii/typography
//! for both egui windows) and [`settings_style`] (the settings form-grid
//! geometry on top).
//!
//! Every value traces to a token: colors through [`super::surfaces`] + the
//! resolved accent, radii through `tokens.radii`, spacing/geometry through
//! `tokens.spacing`, sizes through `tokens.typography.base_size`, and the
//! popup shadow through `tokens.shadows.medium`. The only mode defaults kept
//! are `warn_fg_color`/`error_fg_color` (the palette grows no warning/error
//! tokens yet - recorded deviation, the validation banner inherits egui's
//! per-mode amber/red).

use egui::{
    Color32, Margin, Rounding, Stroke, Style, TextStyle, Visuals,
    style::{Spacing, WidgetVisuals},
};
use flowshot_core::config::UiConfig;
use flowshot_core::tokens::DesignTokens;

use super::{ThemeMode, ink_on, parse_hex_rgba, resolve_colors, surfaces};

/// Inter's line box is ~1.21em; 1.25em is the glyph-height estimate the
/// uniform control height derives from (galley + 2 * vertical button
/// padding), so no widget's text can outgrow the grid row.
const GLYPH_LINE_HEIGHT: f32 = 1.25;
/// The uniform control height's floor width, in em (`DragValue` minimum).
const INTERACT_MIN_WIDTH_EM: f32 = 4.0;
/// Slider track width, in em.
const SLIDER_WIDTH_EM: f32 = 16.0;
/// Slider rail height, in em.
const SLIDER_RAIL_EM: f32 = 0.5;
/// Combo box width, in em.
const COMBO_WIDTH_EM: f32 = 12.0;
/// Single-line text field default width, in em.
const TEXT_FIELD_WIDTH_EM: f32 = 16.0;
/// The check-mark inset box, as a ratio of the checkbox edge.
const ICON_INNER_RATIO: f32 = 0.6;
/// The scrollbar bar width as a multiple of the tight spacing step
/// (small=4 -> 8px).
const SCROLL_BAR_WIDTH: f32 = 2.0;
/// Hover/focus outline tint: accent at this alpha over the surface.
const ACCENT_EDGE_ALPHA: f32 = 0.6;
/// The focus-ring stroke width (every focused widget: fields, buttons,
/// combos, tabs, checkboxes - one consistent ring).
const FOCUS_RING_WIDTH: f32 = 2.0;
/// The text caret width.
const CARET_WIDTH: f32 = 2.0;
/// Hairline stroke width (the egui convention for outlines/separators).
const HAIRLINE: f32 = 1.0;

/// A `u32` token step as points (the palette's steps are byte-sized; a
/// corrupt/oversized token falls back instead of truncating silently).
fn px(value: u32, fallback: u8) -> f32 {
    f32::from(u8::try_from(value).unwrap_or(fallback))
}

/// The uniform control height every settings widget shares: the glyph line
/// box plus the vertical button padding (egui floors buttons, text fields,
/// combos, and sliders at `interact_size.y`, so one value unifies them).
#[must_use]
pub fn control_height(base_size: f32, pad_y: f32) -> f32 {
    base_size * GLYPH_LINE_HEIGHT + 2.0 * pad_y
}

/// Builds the egui style from design tokens + the `[ui]` config group.
///
/// Invalid hex values in `ui` fall back to the token palette (a corrupt
/// color must never panic or block the window - the model's validation
/// flags it for Apply instead).
#[must_use]
pub fn style(tokens: &DesignTokens, ui: &UiConfig, mode: ThemeMode) -> Style {
    let (accent, contrast) = resolve_colors(tokens, ui);
    let s = surfaces(contrast, mode);
    let visuals = visuals(tokens, accent, s, mode);

    let small = px(tokens.spacing.small, 4);
    let medium = px(tokens.spacing.medium, 8);
    let spacing = Spacing {
        item_spacing: egui::vec2(medium, small),
        button_padding: egui::vec2(medium, small),
        window_margin: Margin::same(medium),
        menu_margin: Margin::same(medium),
        ..Spacing::default()
    };

    let mut style = Style {
        visuals,
        spacing,
        ..Style::default()
    };
    let base_size = px(tokens.typography.base_size, 14);
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

/// The form-grid style: [`style`] plus the grid geometry (uniform control
/// height, checkbox icon edge, slider/combo/field widths) that BOTH egui
/// panels project - the settings window and the launcher dialog share one
/// control vocabulary (the dialog's own window size is the only difference).
#[must_use]
pub fn settings_style(tokens: &DesignTokens, ui: &UiConfig, mode: ThemeMode) -> Style {
    let mut style = style(tokens, ui, mode);
    let base_size = px(tokens.typography.base_size, 14);
    let small = px(tokens.spacing.small, 4);
    let large = px(tokens.spacing.large, 16);
    let spacing = &mut style.spacing;
    spacing.interact_size = egui::vec2(
        base_size * INTERACT_MIN_WIDTH_EM,
        control_height(base_size, small),
    );
    spacing.icon_width = base_size;
    spacing.icon_width_inner = base_size * ICON_INNER_RATIO;
    spacing.icon_spacing = small;
    spacing.slider_width = base_size * SLIDER_WIDTH_EM;
    spacing.slider_rail_height = base_size * SLIDER_RAIL_EM;
    spacing.combo_width = base_size * COMBO_WIDTH_EM;
    spacing.text_edit_width = base_size * TEXT_FIELD_WIDTH_EM;
    spacing.indent = large;
    // A solid, reserved, always-drawn scrollbar (egui's default floats
    // invisibly until hovered - no scroll affordance for a settings page).
    // The handle reads against the bar well through `Surfaces::control`.
    // A solid, reserved, always-drawn scrollbar (egui's default floats
    // invisibly until hovered - no scroll affordance for a settings page).
    // The handle reads against the bar well through `Surfaces::control`.
    // Placement is intentional: egui pins the reserved bar to the window's
    // right edge, so the bar OWNS the right window-margin band (browser /
    // GNOME scrollbar-at-window-edge pattern); the panel-colored strip
    // between the content viewport and the bar is the window margin
    // itself, uniform with the other three sides. Floating bars were
    // rejected: they fade out at idle, and `bar_outer_margin` shifts of
    // the reserved bar eat column width without moving the pin.
    spacing.scroll = egui::style::ScrollStyle {
        bar_width: small * SCROLL_BAR_WIDTH,
        ..egui::style::ScrollStyle::solid()
    };
    // No animation: the bar's show/hide fade is `animate_bool` over frame
    // time, so a zero-delta offscreen pair would capture it mid-fade at
    // alpha 0 (and a settings page wants the bar present the frame the
    // content overflows, not fading in).
    style.animation_time = 0.0;
    style
}

/// The token-projected visuals (both modes share the derivation; the
/// surface scale carries the mode split).
fn visuals(tokens: &DesignTokens, accent: Color32, s: super::Surfaces, mode: ThemeMode) -> Visuals {
    let mut visuals = match mode {
        ThemeMode::Dark => Visuals::dark(),
        ThemeMode::Light => Visuals::light(),
    };
    let medium = px(tokens.radii.medium, 4);
    let rounding = Rounding::same(medium);
    let accent_edge = Stroke::new(HAIRLINE, accent.gamma_multiply(ACCENT_EDGE_ALPHA));
    let hairline = |color: Color32| Stroke::new(HAIRLINE, color);

    // Invariant: noninteractive fills stay OPAQUE - egui fades disabled
    // widgets toward `fade_out_to_color()` = `noninteractive.weak_bg_fill`,
    // so a transparent value renders every disabled control invisible.
    // Disabled controls keep the resting button SHAPE (same surface, weaker
    // outline and ink) so they read as inert-but-present, not as holes.
    visuals.widgets.noninteractive = WidgetVisuals {
        bg_fill: s.button,
        weak_bg_fill: s.button,
        bg_stroke: hairline(s.separator),
        rounding,
        fg_stroke: hairline(s.text_weak),
        expansion: 0.0,
    };
    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: s.control,
        weak_bg_fill: s.button,
        bg_stroke: hairline(s.outline),
        rounding,
        fg_stroke: hairline(s.text),
        expansion: 0.0,
    };
    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: s.control_hover,
        weak_bg_fill: s.button_hover,
        bg_stroke: accent_edge,
        rounding,
        fg_stroke: hairline(s.text),
        expansion: 0.0,
    };
    visuals.widgets.active = WidgetVisuals {
        bg_fill: accent,
        weak_bg_fill: accent,
        bg_stroke: hairline(accent),
        rounding,
        fg_stroke: hairline(ink_on(accent)),
        expansion: 0.0,
    };
    visuals.widgets.open = WidgetVisuals {
        bg_fill: s.control,
        weak_bg_fill: s.button_hover,
        bg_stroke: accent_edge,
        rounding,
        fg_stroke: hairline(s.text),
        expansion: 0.0,
    };

    visuals.panel_fill = s.window;
    visuals.window_fill = s.popup;
    visuals.window_stroke = hairline(s.outline_strong);
    visuals.faint_bg_color = s.card;
    visuals.extreme_bg_color = s.field;
    visuals.code_bg_color = s.field;
    visuals.override_text_color = None;
    visuals.selection.bg_fill = accent.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(FOCUS_RING_WIDTH, accent);
    visuals.hyperlink_color = accent;
    visuals.text_cursor.stroke = Stroke::new(CARET_WIDTH, accent);
    visuals.slider_trailing_fill = true;
    visuals.indent_has_left_vline = false;
    visuals.window_rounding = Rounding::same(px(tokens.radii.large, 8));
    visuals.menu_rounding = rounding;
    visuals.popup_shadow = popup_shadow(tokens);
    // Crisp clipping at the scroll viewport edge (the egui default lets
    // scrolled content bleed 6px past the clip for shadow softness).
    visuals.clip_rect_margin = 0.0;
    visuals
}

/// Projects `tokens.shadows.medium` into the egui popup shadow.
fn popup_shadow(tokens: &DesignTokens) -> egui::Shadow {
    let token = &tokens.shadows.medium;
    let color = parse_hex_rgba(&token.color).map_or_else(
        || Color32::from_black_alpha(0x29),
        |[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a),
    );
    egui::Shadow {
        offset: egui::vec2(token.offset[0], token.offset[1]),
        blur: f32::from(u16::try_from(token.blur).unwrap_or(8)),
        spread: 0.0,
        color,
    }
}
