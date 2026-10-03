//! The `[telemetry]` group: the two consent toggles with the first-launch
//! dialog's copy, the "what exactly is sent" disclosure, and the
//! next-start note.
//!
//! `asked_on_first_launch` is the first-launch dialog's bookkeeping flag
//! ([`crate::consent`]) and is deliberately NOT editable here; Reset
//! restores it to false, so a factory reset asks the question once more on
//! the next daemon startup (documented in the consent module header).
//!
//! The disclosure expander starts OPEN: what-is-sent transparency is the
//! card's whole point, and a collapsed header would hide the tier contents
//! behind a click (users can collapse it for the session).

use egui::{CollapsingHeader, RichText, Ui};
use flowshot_core::config::Config;

use crate::settings::fields::toggle;
use crate::settings::form::hint;
use crate::settings::layout::FormMetrics;
use crate::settings::strings;

pub(super) fn show(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut changed = toggle(
        ui,
        m,
        strings::FIELD_TELEMETRY_ENABLED,
        &mut config.telemetry.enabled,
    );
    hint(ui, m, strings::HINT_TELEMETRY_ENABLED);
    changed |= toggle(
        ui,
        m,
        strings::FIELD_TELEMETRY_DETAILS,
        &mut config.telemetry.include_technical_details,
    );
    hint(ui, m, strings::HINT_TELEMETRY_DETAILS);
    disclosure(ui, m);
    hint(ui, m, strings::HINT_TELEMETRY_NEXT_START);
    changed
}

/// The tier disclosure, aligned to the control column (the `hint` /
/// `sub_section` convention: it annotates the toggles above it).
fn disclosure(ui: &mut Ui, m: &FormMetrics) {
    let available = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(m.label_width(available) + m.gutter());
        CollapsingHeader::new(
            RichText::new(strings::LABEL_TELEMETRY_WHAT).color(ui.visuals().weak_text_color()),
        )
        .default_open(true)
        .show(ui, |ui| {
            ui.label(strings::TELEMETRY_TIER1);
            ui.add_space(m.small());
            ui.label(strings::TELEMETRY_TIER2);
        });
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use flowshot_core::config::TelemetryConfig;
    use flowshot_core::tokens::DesignTokens;

    use crate::settings::SettingsModel;
    use crate::settings::layout::FormMetrics;

    #[test]
    fn telemetry_toggles_roundtrip_through_the_save_path() {
        // Given: a model with both telemetry toggles edited
        let mut model = SettingsModel::default();
        model.config_mut().telemetry.enabled = true;
        model.config_mut().telemetry.include_technical_details = true;
        model.mark_dirty();
        // When: serialized (the Apply write) and reloaded
        let text = model.config().to_toml_string().unwrap();
        let reloaded = SettingsModel::from_toml_str(&text);
        // Then: both toggles survive byte-stable and the dialog's
        // bookkeeping flag is untouched by the settings surface
        assert_eq!(
            reloaded.config().telemetry,
            TelemetryConfig {
                enabled: true,
                include_technical_details: true,
                asked_on_first_launch: false,
            }
        );
        assert_eq!(reloaded.config().to_toml_string().unwrap(), text);
    }

    #[test]
    fn telemetry_card_renders_without_spurious_change() {
        // Given: the card body over a default config, headless egui
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::egui_host::theme::fonts());
        let mut model = SettingsModel::default();
        let metrics = FormMetrics::from_tokens(&DesignTokens::default());
        let mut changed = true;
        // When: one frame renders with no input
        ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let config = model.config_mut();
                changed = super::show(ui, &metrics, config);
            });
        })
        .drop_without_applying_deltas();
        // Then: rendering alone never marks the config dirty
        assert!(!changed);
    }
}
