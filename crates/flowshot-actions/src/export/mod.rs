//! Export pipeline: save, encode, open, notify.

mod encode;
mod open;
mod path;
mod pattern;
mod stdout;

pub use encode::{encode, encode_jpeg, encode_png, encode_webp};
pub use open::open_with_app;
pub use path::{next_available_path, resolve_save_path};
pub use pattern::{DEFAULT_PATTERN, expand_pattern, sanitize_filename};
pub use stdout::{format_geometry, write_raw_png};

use std::path::{Path, PathBuf};

use flowshot_core::config::SaveConfig;
use image::DynamicImage;

use crate::error::ExportError;

/// Callback for user-visible notifications (save success, errors).
///
/// Wired by the daemon to the real notification backend.
/// The daemon gates every call through `[daemon].notifications` (the
/// post-capture executor in `crate::clipboard::pipeline` applies the
/// gate for the whole action run).
pub trait NotifySink: Send + Sync {
    /// Called after a successful save.
    fn on_saved(&self, path: &Path);
    /// Called when an action fails.
    fn on_error(&self, message: &str);
    /// Explicit success-toast request (the `notify` action).
    ///
    /// `saved_path` is `Some` when the capture was also saved to disk.
    fn on_success(&self, saved_path: Option<&Path>);
}

/// Callback for file-save dialogs.
///
/// The library never opens a dialog directly; the CLI wires
/// this to `rfd` or another backend.
pub trait FileDialogSink: Send + Sync {
    /// Ask the user for a save path. Returns `None` if cancelled.
    ///
    /// # Errors
    ///
    /// Returns an error if the dialog cannot be shown.
    fn pick_save_path(&self, default_name: &str) -> Result<Option<PathBuf>, ExportError>;
}

/// A no-op [`NotifySink`] for tests and headless operation.
#[derive(Debug, Clone, Copy)]
pub struct NullNotifySink;

impl NotifySink for NullNotifySink {
    fn on_saved(&self, _path: &Path) {}
    fn on_error(&self, _message: &str) {}
    fn on_success(&self, _saved_path: Option<&Path>) {}
}

/// Save an image to disk according to the save configuration.
///
/// # Errors
///
/// Returns [`ExportError`] if the path cannot be resolved, the image
/// cannot be encoded, or the file cannot be written.
pub fn save(
    image: &DynamicImage,
    config: &SaveConfig,
    dialog: &dyn FileDialogSink,
    notify: &dyn NotifySink,
) -> Result<PathBuf, ExportError> {
    let target = resolve_save_path(config, dialog)?;
    let bytes = encode(image, &config.extension, config.jpeg_quality)?;
    std::fs::write(&target, &bytes)?;
    notify.on_saved(&target);
    Ok(target)
}

/// Copy the captured image to the clipboard per `[save]` config: the
/// `copy` action of the post-capture pipeline (`image/png` always,
/// `image/jpeg` appended when `clipboard_format = 'jpeg'`).
///
/// # Errors
///
/// Returns [`crate::error::ClipboardError::Encode`] if encoding fails or
/// [`crate::error::ClipboardError::Transport`] if the clipboard cannot
/// be taken.
pub fn copy_to_clipboard(
    image: &DynamicImage,
    config: &SaveConfig,
    clipboard: &crate::clipboard::Clipboard,
) -> Result<(), crate::error::ClipboardError> {
    clipboard.copy_capture(image, config)
}
