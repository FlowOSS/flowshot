//! The field rows: every config control class as one grid row (label cell +
//! control cell), built on [`super::form`]. Each helper returns the change
//! flag the panel bodies fold into their dirty state - the same contract
//! the pre-rework helpers had. The launcher dialog consumes the same rows
//! (combo / text / swatch), which is what keeps the two panels on one
//! standard.

use std::ops::RangeInclusive;

use egui::{ComboBox, StrokeKind, TextEdit, Ui};

use crate::egui_host::theme::parse_hex_rgb;
use crate::settings::layout::FormMetrics;
use crate::settings::strings;
use crate::settings::window::PathPicker;

use super::form::{TokenCheckbox, row};

/// The hex text field width in em: the 7-character `#RRGGBB` format plus
/// editing headroom.
const HEX_FIELD_EM: f32 = 8.0;
/// The fallback swatch color for malformed hex (the picker's neutral).
const FALLBACK_SWATCH_RGB: [u8; 3] = [0x7F, 0x7F, 0x7F];
/// The hex literal a new palette swatch starts as.
pub(crate) const NEW_SWATCH_HEX: &str = "#7F7F7F";

/// A checkbox row; returns whether the value changed.
pub(crate) fn toggle(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut bool) -> bool {
    row(ui, m, label, |ui| ui.add(TokenCheckbox::new(value))).changed()
}

/// A clamped numeric row (slider + integrated drag value, the token-styled
/// rail/trailing fill); rail + value box together fill the control column
/// so the value box's right edge matches the text fields and combos
/// (egui adds the drag value AFTER the rail, so its width comes off the
/// rail budget). Returns whether the value changed.
pub(crate) fn number<T: egui::emath::Numeric>(
    ui: &mut Ui,
    m: &FormMetrics,
    label: &str,
    range: RangeInclusive<T>,
    value: &mut T,
) -> bool {
    row(ui, m, label, |ui| {
        let value_box = ui.spacing().interact_size.x + ui.spacing().item_spacing.x;
        let rail = (ui.available_width() - value_box).max(m.control_min_width());
        ui.spacing_mut().slider_width = rail;
        ui.add(egui::Slider::new(value, range).integer())
    })
    .changed()
}

/// The shared single-line field projection: token padding, the uniform
/// control height floor, and an explicit width. `width` is the field's
/// OUTER budget - `desired_width` is the inner text width, so the margins
/// come off first (forgetting this overflows the grid cell by the margin
/// sum and egui's placer widens the whole card in a chain).
pub(crate) fn text_edit<'t>(ui: &Ui, value: &'t mut String, width: f32) -> TextEdit<'t> {
    let margins = 2.0 * ui.spacing().button_padding.x;
    TextEdit::singleline(value)
        .margin(ui.spacing().button_padding)
        .min_size(ui.spacing().interact_size)
        .desired_width((width - margins).max(1.0))
}

/// A single-line text row bound directly to the config string, capped at
/// [`FormMetrics::field_max_width`] so short values keep an honest width;
/// returns whether the value changed.
pub(crate) fn text_field(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut String) -> bool {
    row(ui, m, label, |ui| {
        let width = ui
            .available_width()
            .min(m.field_max_width())
            .max(m.control_min_width());
        ui.add(text_edit(ui, value, width)).changed()
    })
}

/// The color swatch picker button: egui's popup-backed color edit writing
/// canonical uppercase hex, framed with the strong outline so dark swatches
/// stay visible on dark cards. Returns whether the value changed.
pub(crate) fn swatch(ui: &mut Ui, m: &FormMetrics, value: &mut String) -> bool {
    let mut rgb = parse_hex_rgb(value).unwrap_or(FALLBACK_SWATCH_RGB);
    let response = ui.color_edit_button_srgb(&mut rgb);
    let changed = response.changed();
    if changed {
        *value = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
    }
    ui.painter().rect_stroke(
        response.rect,
        m.control_radius(),
        ui.visuals().window_stroke,
        StrokeKind::Middle,
    );
    changed
}

/// A `#RRGGBB` color row: swatch picker plus a compact hex text field sized
/// for the 7-character format (validation flags malformed input); returns
/// whether the value changed.
pub(crate) fn hex_color(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut String) -> bool {
    row(ui, m, label, |ui| {
        let mut changed = swatch(ui, m, value);
        let hex_width = m.base_size() * HEX_FIELD_EM;
        let remaining =
            (ui.available_width() - ui.spacing().item_spacing.x).max(m.control_min_width());
        changed |= ui
            .add(text_edit(ui, value, hex_width.min(remaining)))
            .changed();
        changed
    })
}

/// A combo row over `(value, label)` options, capped at
/// [`FormMetrics::combo_max_width`]; returns whether the selection changed.
pub(crate) fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    m: &FormMetrics,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let selected = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or(label, |(_, text)| *text);
    row(ui, m, label, |ui| {
        ComboBox::new(label, "")
            .width(ui.available_width().min(m.combo_max_width()))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (option, text) in options {
                    ui.selectable_value(value, *option, *text);
                }
            })
            .response
            .changed()
    })
}

/// The save-path row: Browse is laid out FIRST from the cell's right edge
/// (right-to-left pass, so the grid's right edge holds without font
/// measurement), then the text field fills what remains to the left.
/// Browse is enabled only while the binary layer wired the rfd picker
/// seam. Returns whether the path changed.
pub(crate) fn path_field(
    ui: &mut Ui,
    m: &FormMetrics,
    value: &mut String,
    picker: Option<&PathPicker>,
) -> bool {
    row(ui, m, strings::FIELD_SAVE_PATH, |ui| {
        let (edited, selection) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let browse =
                    ui.add_enabled(picker.is_some(), egui::Button::new(strings::BUTTON_BROWSE));
                let width = ui.available_width().max(m.control_min_width());
                let edited = ui.add(text_edit(ui, value, width)).changed();
                let selection = if browse.clicked() {
                    picker.and_then(PathPicker::pick)
                } else {
                    None
                };
                (edited, selection)
            })
            .inner;
        if let Some(selection) = selection {
            *value = selection;
            return true;
        }
        edited
    })
}
