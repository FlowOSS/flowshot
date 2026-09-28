//! The field rows: every config control class as one grid row (label cell +
//! control cell), built on [`super::form`]. Each helper returns the change
//! flag the tab bodies fold into `SettingsModel::mark_dirty` - the same
//! contract the pre-rework helpers had.

use std::ops::RangeInclusive;

use egui::{ComboBox, TextEdit, Ui};

use crate::egui_host::theme::parse_hex_rgb;
use crate::settings::layout::FormMetrics;
use crate::settings::strings;
use crate::settings::window::PathPicker;

use super::form::{TokenCheckbox, row};

/// The hex text field width in em: the 7-character `#RRGGBB` format plus
/// editing headroom.
const HEX_FIELD_EM: f32 = 8.0;

/// A checkbox row; returns whether the value changed.
pub(super) fn toggle(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut bool) -> bool {
    row(ui, m, label, |ui| ui.add(TokenCheckbox::new(value))).changed()
}

/// A clamped numeric row (slider + integrated drag value, the token-styled
/// rail/trailing fill); rail + value box together fill the control column
/// so the value box's right edge matches the text fields and combos
/// (egui adds the drag value AFTER the rail, so its width comes off the
/// rail budget). Returns whether the value changed.
pub(super) fn number<T: egui::emath::Numeric>(
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
fn text_edit<'t>(ui: &Ui, value: &'t mut String, width: f32) -> TextEdit<'t> {
    let margins = 2.0 * ui.spacing().button_padding.x;
    TextEdit::singleline(value)
        .margin(ui.spacing().button_padding)
        .min_size(ui.spacing().interact_size)
        .desired_width((width - margins).max(1.0))
}

/// A single-line text row bound directly to the config string, filling the
/// control column; returns whether the value changed.
pub(super) fn text_field(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut String) -> bool {
    row(ui, m, label, |ui| {
        let width = ui.available_width().max(m.control_min_width());
        ui.add(text_edit(ui, value, width)).changed()
    })
}

/// A `#RRGGBB` color row: swatch picker (writes canonical uppercase hex)
/// plus a compact hex text field sized for the 7-character format
/// (validation flags malformed input); returns whether the value changed.
pub(super) fn hex_color(ui: &mut Ui, m: &FormMetrics, label: &str, value: &mut String) -> bool {
    row(ui, m, label, |ui| {
        let mut rgb = parse_hex_rgb(value).unwrap_or([0x7F, 0x7F, 0x7F]);
        let mut changed = ui.color_edit_button_srgb(&mut rgb).changed();
        if changed {
            *value = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
        }
        let hex_width = m.base_size() * HEX_FIELD_EM;
        let remaining =
            (ui.available_width() - ui.spacing().item_spacing.x).max(m.control_min_width());
        changed |= ui
            .add(text_edit(ui, value, hex_width.min(remaining)))
            .changed();
        changed
    })
}

/// A combo row over `(value, label)` options; returns whether the selection
/// changed.
pub(super) fn combo<T: PartialEq + Copy>(
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
        ComboBox::from_id_source(label)
            .width(ui.available_width())
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
pub(super) fn path_field(
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
