//! The `[upload]` group, including the empty-client-id hint (the todo-31
//! CLI gate surfaced where the user can fix it).

use egui::Ui;
use flowshot_core::config::Config;

use crate::settings::strings;
use crate::settings::tabs::{drag_u32, text_field, toggle};

pub(super) fn show(ui: &mut Ui, config: &mut Config) -> bool {
    let mut changed = false;
    changed |= text_field(
        ui,
        strings::FIELD_UPLOAD_PROVIDER,
        &mut config.upload.provider,
    );
    changed |= text_field(
        ui,
        strings::FIELD_UPLOAD_CLIENT_ID,
        &mut config.upload.client_id,
    );
    if config.upload.client_id.is_empty() {
        ui.label(egui::RichText::new(strings::HINT_UPLOAD_UNCONFIGURED).weak());
    }
    changed |= toggle(
        ui,
        strings::FIELD_UPLOAD_NO_CONFIRM,
        &mut config.upload.without_confirmation,
    );
    changed |= toggle(
        ui,
        strings::FIELD_UPLOAD_COPY_URL,
        &mut config.upload.copy_url,
    );
    changed |= drag_u32(
        ui,
        strings::FIELD_UPLOAD_HISTORY_MAX,
        0..=1000,
        &mut config.upload.history_max,
    );
    changed
}
