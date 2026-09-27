//! The four settings tabs (F12 parity: General / Interface / Filename
//! Editor / Shortcuts) plus the shared widget helpers every tab builds on.
//!
//! Tabs are pure immediate-mode projections of [`SettingsModel`]: they read
//! and mutate the model in place and report change through the return
//! value - no widget state lives here beyond egui's own id-keyed memory.

mod filename;
pub use filename::preview_filename;
mod general;
mod interface;
mod shortcuts;

use egui::Ui;

use super::model::{Banner, SettingsModel, Tab};
use super::strings;
use super::theme::ThemeMode;
use super::window::PathPicker;

/// What a rendered frame asks the window layer to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameAction {
    /// Nothing; keep editing.
    #[default]
    None,
    /// Validate + persist + notify (Apply button).
    Apply,
    /// Close the window.
    Close,
}

impl FrameAction {
    /// Combines two actions from the same frame (Close wins over Apply).
    #[must_use]
    pub const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Close, _) | (_, Self::Close) => Self::Close,
            (Self::Apply, _) | (_, Self::Apply) => Self::Apply,
            (Self::None, Self::None) => Self::None,
        }
    }
}

/// Per-frame context the tabs need beyond the model.
#[derive(Debug)]
pub struct TabContext<'a> {
    /// The resolved system dark/light preference (binary layer, ashpd
    /// Settings portal) that `ThemeChoice::System` defers to.
    pub system_theme: ThemeMode,
    /// The save-path dialog seam (binary layer wires rfd); `None` disables
    /// the Browse button.
    pub path_picker: Option<&'a PathPicker>,
}

/// Renders the tab bar, the active tab body, and the bottom action bar;
/// returns the frame's window-level action.
pub(super) fn show(
    ui: &mut Ui,
    model: &mut SettingsModel,
    context: &TabContext<'_>,
) -> FrameAction {
    ui.horizontal(|ui| {
        for tab in Tab::ALL {
            ui.selectable_value(model.active_tab_mut(), tab, tab.label());
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match model.active_tab() {
        Tab::General => general::show(ui, model, context),
        Tab::Interface => interface::show(ui, model),
        Tab::Filename => filename::show(ui, model),
        Tab::Shortcuts => shortcuts::show(ui, model),
    });
    ui.separator();
    show_action_bar(ui, model)
}

/// The banner + Apply/Reset/Close bar; returns the requested action.
fn show_action_bar(ui: &mut Ui, model: &mut SettingsModel) -> FrameAction {
    if let Some(banner) = model.banner().cloned() {
        let message = match &banner {
            Banner::CorruptConfig => strings::BANNER_CORRUPT_CONFIG.to_owned(),
            Banner::SaveFailed(detail) => format!("{}: {detail}", strings::BANNER_SAVE_FAILED),
            Banner::Validation => strings::BANNER_VALIDATION.to_owned(),
        };
        ui.colored_label(ui.visuals().warn_fg_color, message);
    }
    let mut action = FrameAction::None;
    let issues = model.validate();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                model.is_dirty() && issues.is_empty(),
                egui::Button::new(strings::BUTTON_APPLY),
            )
            .clicked()
        {
            action = FrameAction::Apply;
        }
        if ui
            .add_enabled(model.is_dirty(), egui::Button::new(strings::BUTTON_RESET))
            .clicked()
        {
            model.reset();
        }
        for issue in &issues {
            ui.colored_label(ui.visuals().warn_fg_color, issue.to_string());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(strings::BUTTON_CLOSE).clicked() {
                action = FrameAction::Close;
            }
        });
    });
    action
}

/// A checkbox row; returns whether the value changed.
pub(super) fn toggle(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
    ui.checkbox(value, label).changed()
}

/// A default-open collapsing group; returns the body's change flag (`None`
/// body = collapsed this frame = no change).
pub(super) fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui) -> bool) -> bool {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, body)
        .body_returned
        .unwrap_or(false)
}

/// A clamped integer drag row; returns whether the value changed.
pub(super) fn drag_u32(
    ui: &mut Ui,
    label: &str,
    range: std::ops::RangeInclusive<u32>,
    value: &mut u32,
) -> bool {
    ui.add(
        egui::DragValue::new(value)
            .range(range)
            .prefix(format!("{label}: ")),
    )
    .changed()
}

/// A clamped `u8` drag row; returns whether the value changed.
pub(super) fn drag_u8(
    ui: &mut Ui,
    label: &str,
    range: std::ops::RangeInclusive<u8>,
    value: &mut u8,
) -> bool {
    ui.add(
        egui::DragValue::new(value)
            .range(range)
            .prefix(format!("{label}: ")),
    )
    .changed()
}

/// A single-line text row bound directly to the config string; returns
/// whether the value changed.
pub(super) fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value).changed()
    })
    .inner
}

/// A `#RRGGBB` color row: swatch picker (writes canonical uppercase hex)
/// plus a free-text field (validation flags malformed input); returns
/// whether the value changed.
pub(super) fn hex_color(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut rgb = super::theme::parse_hex_rgb(value).unwrap_or([0x7F, 0x7F, 0x7F]);
        let mut changed = ui.color_edit_button_srgb(&mut rgb).changed();
        if changed {
            *value = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
        }
        changed |= ui.text_edit_singleline(value).changed();
        changed
    })
    .inner
}

/// A combo row over `(value, label)` options; returns whether the selection
/// changed.
pub(super) fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let selected = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or(label, |(_, text)| *text);
    egui::ComboBox::from_label(label)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (option, text) in options {
                ui.selectable_value(value, *option, *text);
            }
        })
        .response
        .changed()
}
