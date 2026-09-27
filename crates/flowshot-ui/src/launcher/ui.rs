//! The launcher dialog's widget layer: an immediate-mode projection of
//! [`LauncherModel`] (the settings-tabs pattern). Pure egui - headless
//! testable through [`egui::Context::run`] with synthetic raw input, and
//! the single producer of [`LauncherAction`].

use egui::Ui;

use super::model::{LauncherModel, Target};
use super::request::{GeometryIssue, LauncherRequest};
use super::strings;

/// What a rendered launcher frame asks the window layer to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LauncherAction {
    /// Nothing; keep editing.
    #[default]
    None,
    /// Dispatch the typed request through the binary layer's callback and
    /// close (the Capture button).
    Capture(LauncherRequest),
    /// Close without dispatching (Cancel, Esc, or the window close).
    Cancel,
}

/// The stable widget id of the geometry field (the open-focus target).
fn geometry_id() -> egui::Id {
    egui::Id::new("launcher-geometry")
}

/// Renders the dialog body; returns the frame's window-level action.
pub(super) fn show(ui: &mut Ui, model: &mut LauncherModel) -> LauncherAction {
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        return LauncherAction::Cancel;
    }
    // The standard dialog convention: Enter in the geometry field activates
    // the default button (Capture) when the dialog has a valid request.
    // Checked BEFORE the widgets render so the TextEdit's own Enter
    // handling (surrender focus) cannot eat the frame.
    if ui.memory(|memory| memory.has_focus(geometry_id()))
        && ui.input(|input| input.key_pressed(egui::Key::Enter))
        && let Some(request) = model.request()
    {
        return LauncherAction::Capture(request);
    }
    egui::Grid::new("launcher-grid")
        .num_columns(2)
        .spacing([12.0, 10.0])
        .show(ui, |ui| {
            target_row(ui, model);
            ui.end_row();
            geometry_row(ui, model);
            ui.end_row();
            delay_row(ui, model);
            ui.end_row();
        });
    ui.separator();
    let action = button_row(ui, model);
    if model.take_focus() {
        ui.memory_mut(|memory| memory.request_focus(geometry_id()));
    }
    action
}

fn target_row(ui: &mut Ui, model: &mut LauncherModel) {
    ui.label(strings::TARGET_LABEL);
    let selected = match model.target() {
        Target::Manual => strings::TARGET_MANUAL.to_owned(),
        Target::Monitor(screen) => model
            .monitors()
            .iter()
            .find(|entry| entry.screen == screen)
            .map_or_else(
                || format!("{} {screen}", strings::TARGET_SCREEN_PREFIX),
                |entry| entry.label.clone(),
            ),
    };
    let entries = model.monitors().to_vec();
    egui::ComboBox::from_id_source("launcher-target")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            ui.selectable_value(model.target_mut(), Target::Manual, strings::TARGET_MANUAL);
            for entry in &entries {
                ui.selectable_value(
                    model.target_mut(),
                    Target::Monitor(entry.screen),
                    &entry.label,
                );
            }
        });
}

fn geometry_row(ui: &mut Ui, model: &mut LauncherModel) {
    ui.label(strings::GEOMETRY_LABEL);
    let manual = model.target() == Target::Manual;
    ui.add_enabled_ui(manual, |ui| {
        ui.vertical(|ui| {
            ui.add(
                egui::TextEdit::singleline(model.geometry_text_mut())
                    .id(geometry_id())
                    .desired_width(220.0)
                    .hint_text(strings::GEOMETRY_HINT),
            );
            // Inline validation: the empty field carries the grammar hint as
            // its placeholder; a malformed entry gets the visible error.
            if let Err(issue @ GeometryIssue::Malformed) = model.geometry() {
                ui.colored_label(ui.visuals().error_fg_color, issue.to_string());
            }
        });
    });
}

fn delay_row(ui: &mut Ui, model: &mut LauncherModel) {
    ui.label(strings::DELAY_LABEL);
    ui.add(
        egui::DragValue::new(model.delay_ms_mut())
            .range(0.0..=f64::from(u32::MAX))
            .suffix(strings::DELAY_SUFFIX),
    );
}

fn button_row(ui: &mut Ui, model: &LauncherModel) -> LauncherAction {
    let request = model.request();
    let mut action = LauncherAction::None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(request.is_some(), egui::Button::new(strings::CAPTURE))
            .clicked()
            && let Some(request) = request
        {
            action = LauncherAction::Capture(request);
        }
        if ui.button(strings::CANCEL).clicked() {
            action = LauncherAction::Cancel;
        }
    });
    action
}
