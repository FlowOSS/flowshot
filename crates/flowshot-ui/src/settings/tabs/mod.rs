//! The four settings tabs (F12 parity: General / Interface / Filename
//! Editor / Shortcuts) over the shared form primitives
//! ([`super::form`] / [`super::fields`]) every egui panel builds on.
//!
//! Tabs are pure immediate-mode projections of [`SettingsModel`]: they read
//! and mutate the model in place and report change through the return
//! value - no widget state lives here beyond egui's own id-keyed memory.
//!
//! The window layout: a pill tab bar over a header rule, a scroll region
//! carrying the active tab's centered section cards, and a bottom action
//! bar that stays visible while the cards scroll (the bar is a docked
//! in-panel strip, not a row after the scroll area - the pre-rework layout
//! pushed it off-window whenever the content overflowed).

mod filename;
mod general;
mod interface;
mod shortcuts;

pub use filename::preview_filename;

use egui::{Align, Frame, Layout, Margin, TopBottomPanel, Ui};

use super::form::{pill_tab, primary_button};
use super::layout::FormMetrics;
use super::model::{Banner, SettingsModel, Tab};
use super::strings;
use super::window::PathPicker;
use crate::egui_host::theme::ThemeMode;

/// The banner frame's background tint: the warning ink at this alpha.
const BANNER_TINT_ALPHA: f32 = 0.12;

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
    /// The token-derived form-grid metrics every row/card is laid out on
    /// (the same numbers the offscreen pixel asserts consume).
    pub metrics: FormMetrics,
}

/// Renders the tab bar, the active tab body, and the bottom action bar;
/// returns the frame's window-level action.
pub(super) fn show(
    ui: &mut Ui,
    model: &mut SettingsModel,
    context: &TabContext<'_>,
) -> FrameAction {
    let m = &context.metrics;
    ui.horizontal(|ui| {
        for tab in Tab::ALL {
            let selected = model.active_tab() == tab;
            if pill_tab(ui, m, selected, tab.label()).clicked() {
                *model.active_tab_mut() = tab;
            }
        }
    });
    ui.add_space(m.small());
    ui.separator();
    ui.add_space(m.medium());
    let bar = TopBottomPanel::bottom("settings-action-bar")
        .resizable(false)
        .frame(Frame::none().inner_margin(Margin {
            left: 0.0,
            right: 0.0,
            top: m.medium(),
            bottom: 0.0,
        }));
    let mut action = FrameAction::None;
    bar.show_inside(ui, |ui| {
        action = show_action_bar(ui, m, model);
    });
    egui::ScrollArea::vertical()
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| match model.active_tab() {
            Tab::General => general::show(ui, model, context),
            Tab::Interface => interface::show(ui, model, context),
            Tab::Filename => filename::show(ui, model, context),
            Tab::Shortcuts => shortcuts::show(ui, model, context),
        });
    action
}

/// The banner + Apply/Reset/Close bar; returns the requested action.
fn show_action_bar(ui: &mut Ui, m: &FormMetrics, model: &mut SettingsModel) -> FrameAction {
    if let Some(banner) = model.banner().cloned() {
        let message = match &banner {
            Banner::CorruptConfig => strings::BANNER_CORRUPT_CONFIG.to_owned(),
            Banner::SaveFailed(detail) => format!("{}: {detail}", strings::BANNER_SAVE_FAILED),
            Banner::Validation => strings::BANNER_VALIDATION.to_owned(),
        };
        let tint = ui.visuals().warn_fg_color.gamma_multiply(BANNER_TINT_ALPHA);
        Frame::none()
            .fill(tint)
            .rounding(egui::Rounding::same(m.control_radius()))
            .inner_margin(Margin::same(m.small()))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.colored_label(ui.visuals().warn_fg_color, message);
            });
        ui.add_space(m.small());
    }
    let mut action = FrameAction::None;
    let issues = model.validate();
    ui.horizontal(|ui| {
        if primary_button(ui, m, strings::BUTTON_APPLY, model.apply_enabled()).clicked() {
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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(strings::BUTTON_CLOSE).clicked() {
                action = FrameAction::Close;
            }
        });
    });
    action
}
