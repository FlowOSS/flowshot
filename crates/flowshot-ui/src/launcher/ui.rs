//! The launcher dialog's widget layer: an immediate-mode projection of
//! [`LauncherModel`] (the settings-tabs pattern) over the SAME form
//! vocabulary the settings window uses ([`crate::settings::form`] rows +
//! [`crate::settings::fields`] controls + the token metrics), so both egui
//! panels share one standard. Pure egui - headless testable through
//! [`egui::Context::run`] with synthetic raw input, and the single producer
//! of [`LauncherAction`].

use egui::Ui;

use crate::settings::FormMetrics;
use crate::settings::fields::{combo, text_edit};
use crate::settings::form::{error_hint, primary_button, row};

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
pub(super) fn show(
    ui: &mut Ui,
    model: &mut LauncherModel,
    metrics: &FormMetrics,
) -> LauncherAction {
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
    target_row(ui, model, metrics);
    geometry_row(ui, model, metrics);
    row(ui, metrics, strings::DELAY_LABEL, |ui| {
        ui.add(
            egui::DragValue::new(model.delay_ms_mut())
                .range(0.0..=f64::from(u32::MAX))
                .suffix(strings::DELAY_SUFFIX),
        );
    });
    ui.add_space(metrics.small());
    ui.separator();
    let action = button_row(ui, model, metrics);
    if model.take_focus() {
        ui.memory_mut(|memory| memory.request_focus(geometry_id()));
    }
    action
}

fn target_row(ui: &mut Ui, model: &mut LauncherModel, m: &FormMetrics) {
    // The monitor labels are borrowed from a clone so the combo can hold a
    // mutable borrow of the target at the same time.
    let entries = model.monitors().to_vec();
    let mut options: Vec<(Target, &str)> = vec![(Target::Manual, strings::TARGET_MANUAL)];
    options.extend(
        entries
            .iter()
            .map(|entry| (Target::Monitor(entry.screen), entry.label.as_str())),
    );
    combo(ui, m, strings::TARGET_LABEL, model.target_mut(), &options);
}

fn geometry_row(ui: &mut Ui, model: &mut LauncherModel, m: &FormMetrics) {
    let manual = model.target() == Target::Manual;
    row(ui, m, strings::GEOMETRY_LABEL, |ui| {
        ui.add_enabled_ui(manual, |ui| {
            let width = ui.available_width().max(m.control_min_width());
            ui.add(
                text_edit(ui, model.geometry_text_mut(), width)
                    .id(geometry_id())
                    .hint_text(strings::GEOMETRY_HINT),
            );
        });
    });
    // Inline validation: the empty field carries the grammar hint as its
    // placeholder; a malformed entry gets the visible error row.
    if let Err(issue @ GeometryIssue::Malformed) = model.geometry() {
        error_hint(ui, m, &issue.to_string());
    }
}

fn button_row(ui: &mut Ui, model: &LauncherModel, m: &FormMetrics) -> LauncherAction {
    let request = model.request();
    let mut action = LauncherAction::None;
    ui.horizontal(|ui| {
        if primary_button(ui, m, strings::CAPTURE, request.is_some()).clicked()
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
