//! The `[upload]` group, including the empty-client-id hint (the
//! upload CLI gate surfaced where the user can fix it).

use egui::Ui;
use flowshot_core::config::Config;

use crate::settings::fields::{number, text_field, toggle};
use crate::settings::form::hint;
use crate::settings::layout::FormMetrics;
use crate::settings::strings;

pub(super) fn show(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut changed = text_field(
        ui,
        m,
        strings::FIELD_UPLOAD_PROVIDER,
        &mut config.upload.provider,
    );
    changed |= text_field(
        ui,
        m,
        strings::FIELD_UPLOAD_CLIENT_ID,
        &mut config.upload.client_id,
    );
    if config.upload.client_id.is_empty() {
        hint(ui, m, strings::HINT_UPLOAD_UNCONFIGURED);
    }
    changed |= toggle(
        ui,
        m,
        strings::FIELD_UPLOAD_NO_CONFIRM,
        &mut config.upload.without_confirmation,
    );
    changed |= toggle(
        ui,
        m,
        strings::FIELD_UPLOAD_COPY_URL,
        &mut config.upload.copy_url,
    );
    changed |= number(
        ui,
        m,
        strings::FIELD_UPLOAD_HISTORY_MAX,
        0..=1000,
        &mut config.upload.history_max,
    );
    changed
}
