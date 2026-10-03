//! The consent dialog's widget layer: an immediate-mode projection of
//! [`ConsentModel`] over the SAME form vocabulary the settings window and
//! the launcher dialog use ([`TokenCheckbox`] + [`primary_button`] + the
//! token metrics). Pure egui - headless testable through
//! [`egui::Context::run`] with synthetic raw input, and the single
//! producer of [`ConsentAction`].

use egui::{Key, Label, Sense, TextWrapMode, Ui};

use flowshot_core::config::TelemetryConfig;

use crate::settings::FormMetrics;
use crate::settings::form::{TokenCheckbox, primary_button};

use super::model::ConsentModel;
use super::strings;

/// What a rendered consent frame asks the window layer to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsentAction {
    /// Nothing; keep editing.
    #[default]
    None,
    /// Record the checkbox answers ("Save choice" button or Enter);
    /// carries the config write for the persistence seam.
    Save(TelemetryConfig),
    /// Record the deferred choice ("Not now" button, Esc, or the window
    /// close - the window layer maps every dismissal here).
    NotNow,
}

/// Renders the dialog body; returns the frame's window-level action.
pub(super) fn show(ui: &mut Ui, model: &mut ConsentModel, m: &FormMetrics) -> ConsentAction {
    // Keyboard shortcuts are checked BEFORE the widgets render (the
    // launcher's Enter convention): Esc defers, Enter saves the current
    // answers - both record the choice exactly once.
    if ui.input(|input| input.key_pressed(Key::Escape)) {
        return ConsentAction::NotNow;
    }
    if ui.input(|input| input.key_pressed(Key::Enter)) {
        return ConsentAction::Save(model.saved_choice());
    }
    ui.label(strings::BODY);
    ui.add_space(m.medium());
    option_row(ui, model.send_mut(), strings::OPTION_SEND);
    option_row(ui, model.details_mut(), strings::OPTION_DETAILS);
    ui.add_space(m.medium());
    ui.separator();
    button_row(ui, m, model)
}

/// One consent option: the token checkbox plus its full-copy label; the
/// label is click-through, so the whole sentence toggles the box.
fn option_row(ui: &mut Ui, value: &mut bool, text: &str) {
    ui.horizontal(|ui| {
        ui.add(TokenCheckbox::new(value));
        let response = ui.add(
            Label::new(text)
                .wrap_mode(TextWrapMode::Wrap)
                .sense(Sense::click()),
        );
        if response.clicked() {
            *value = !*value;
        }
    });
}

fn button_row(ui: &mut Ui, m: &FormMetrics, model: &ConsentModel) -> ConsentAction {
    let mut action = ConsentAction::None;
    ui.horizontal(|ui| {
        // "Save choice" is always enabled: every checkbox combination is a
        // valid answer (both-off IS the opt-out).
        if primary_button(ui, m, strings::SAVE_CHOICE, true).clicked() {
            action = ConsentAction::Save(model.saved_choice());
        }
        if ui.button(strings::NOT_NOW).clicked() {
            action = ConsentAction::NotNow;
        }
    });
    action
}
