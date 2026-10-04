//! The consent dialog's widget layer: an immediate-mode projection of
//! [`ConsentModel`] over the SAME form vocabulary the settings window and
//! the launcher dialog use ([`TokenCheckbox`] + [`primary_button`] + the
//! settings card surface + the token metrics). Pure egui - headless
//! testable through [`egui::Context::run`] with synthetic raw input, and
//! the single producer of [`ConsentAction`].
//!
//! The layout is the designed first-run surface: a centered brand header
//! (logo mark, semibold title, muted pitch), the two consent options as a
//! raised card of label + hint rows, and a right-aligned action row with
//! the primary button last (the dialog convention).

use egui::{Align, Frame, Key, Label, Layout, RichText, Sense, TextStyle, TextWrapMode, Ui};

use flowshot_core::config::TelemetryConfig;

use crate::egui_host::theme;
use crate::settings::FormMetrics;
use crate::settings::form::{TokenCheckbox, primary_button};

use super::brand;
use super::model::ConsentModel;
use super::strings;

/// What a rendered consent frame asks the window layer to do.
///
/// The re-ask contract (USER DIRECTIVE 2026-10-04, module docs): ONLY
/// [`Self::Save`] settles the question. Every dismissal - the "Not now"
/// button, Esc, or the window close - is [`Self::Dismiss`]: it closes the
/// dialog and persists NOTHING, so the next daemon start asks again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsentAction {
    /// Nothing; keep editing.
    #[default]
    None,
    /// Record the checkbox answers ("Save choice" button or Enter);
    /// carries the config write for the persistence seam - the ONLY
    /// action that settles the prompt.
    Save(TelemetryConfig),
    /// Close without recording ("Not now" button, Esc, or the window
    /// close - the window layer maps every dismissal here). No config
    /// write: `asked_on_first_launch` stays false and the daemon-startup
    /// prompt re-arms. The settings Telemetry card is the permanent
    /// control; the dialog deliberately has no "don't ask again".
    Dismiss,
}

impl ConsentAction {
    /// The config write this action carries: [`Self::Save`] is the only
    /// persisting action - [`Self::Dismiss`] and [`Self::None`] write
    /// nothing (the re-ask contract). The frame dispatcher routes through
    /// this accessor, so the unit tests pin the persistence seam itself.
    #[must_use]
    pub const fn persisted(&self) -> Option<&TelemetryConfig> {
        match self {
            Self::None | Self::Dismiss => None,
            Self::Save(choice) => Some(choice),
        }
    }
}

/// Renders the dialog body; returns the frame's window-level action.
pub(super) fn show(ui: &mut Ui, model: &mut ConsentModel, m: &FormMetrics) -> ConsentAction {
    // Keyboard shortcuts are checked BEFORE the widgets render (the
    // launcher's Enter convention): Esc dismisses (records nothing),
    // Enter saves the current answers.
    if ui.input(|input| input.key_pressed(Key::Escape)) {
        return ConsentAction::Dismiss;
    }
    if ui.input(|input| input.key_pressed(Key::Enter)) {
        return ConsentAction::Save(model.saved_choice());
    }
    header(ui, m);
    ui.add_space(m.large());
    option_card(ui, model, m);
    ui.add_space(m.large());
    button_row(ui, m, model)
}

/// The centered brand header: the logo mark, the semibold title, and the
/// pitch paragraph in muted ink.
fn header(ui: &mut Ui, m: &FormMetrics) {
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        brand::mark(ui, brand::mark_edge(m.base_size()));
        ui.add_space(m.medium());
        ui.label(
            RichText::new(strings::WINDOW_TITLE)
                .font(theme::semibold(m.title_size()))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(m.small());
        ui.label(
            RichText::new(strings::BODY)
                .color(ui.visuals().weak_text_color())
                .text_style(TextStyle::Body),
        );
    });
}

/// The two consent options on the settings card surface (raised fill,
/// token radius + padding), each a click-through label + hint row.
fn option_card(ui: &mut Ui, model: &mut ConsentModel, m: &FormMetrics) {
    let inner = ui.available_width() - 2.0 * m.card_padding();
    Frame::NONE
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(m.card_radius())
        .inner_margin(m.card_padding())
        .show(ui, |ui| {
            ui.set_min_width(inner);
            option_row(
                ui,
                m,
                model.send_mut(),
                strings::OPTION_SEND_LABEL,
                strings::OPTION_SEND_HINT,
            );
            ui.add_space(m.medium());
            option_row(
                ui,
                m,
                model.details_mut(),
                strings::OPTION_DETAILS_LABEL,
                strings::OPTION_DETAILS_HINT,
            );
        });
}

/// One consent option: the token checkbox beside a medium-weight label and
/// a muted wrapped hint; label and hint are click-through, so the whole
/// sentence toggles the box (the settings checkbox convention).
fn option_row(ui: &mut Ui, m: &FormMetrics, value: &mut bool, label: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.add(TokenCheckbox::new(value));
        ui.vertical(|ui| {
            let label_response = ui.add(
                Label::new(RichText::new(label).font(theme::medium(m.base_size())))
                    .wrap_mode(TextWrapMode::Wrap)
                    .sense(Sense::click()),
            );
            let hint_response = ui.add(
                Label::new(
                    RichText::new(hint)
                        .text_style(TextStyle::Small)
                        .color(ui.visuals().weak_text_color()),
                )
                .wrap_mode(TextWrapMode::Wrap)
                .sense(Sense::click()),
            );
            if (label_response | hint_response).clicked() {
                *value = !*value;
            }
        });
    });
}

/// The action row, right-aligned with the primary last (the dialog
/// convention): "Save choice" is always enabled - every checkbox
/// combination is a valid answer (both-off IS the opt-out). The row height
/// is FIXED at the control height: a bare `with_layout(right_to_left(..))`
/// child would center its buttons in the whole remaining window height and
/// drift the row down as the window grows.
fn button_row(ui: &mut Ui, m: &FormMetrics, model: &ConsentModel) -> ConsentAction {
    let mut action = ConsentAction::None;
    let size = egui::vec2(ui.available_width(), m.control_height());
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        if primary_button(ui, m, strings::SAVE_CHOICE, true).clicked() {
            action = ConsentAction::Save(model.saved_choice());
        }
        if ui.button(strings::NOT_NOW).clicked() {
            action = ConsentAction::Dismiss;
        }
    });
    action
}
