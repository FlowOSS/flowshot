//! Pin actions and the multi-pin registry.
//!
//! # Crate placement
//!
//! Pin WINDOWS are GPU-rendered winit surfaces and live in
//! `flowshot_ui::pins`; this module holds the two platform-service halves
//! of the action layer:
//!
//! - [`PinRegistry`] - the multi-pin bookkeeping that feeds the daemon
//!   lifecycle ("pins alive" persistence reason; the smart-lifecycle
//!   replacement for the dropped `autoCloseIdleDaemon` flag).
//! - [`copy_pin`] / [`save_pin`] - the context-menu action implementations
//!   on the clipboard and export seams.
//!
//! The UI crate cannot depend on this one (purity gate: this crate is
//! platform-native by design - it owns the Wayland and X11 clipboard
//! backends), so the UI exposes the
//! `flowshot_ui::pins::PinActionSink` callback trait and the BINARY layer
//! (CLI / daemon) bridges the two: it converts the UI's
//! `PinSnapshot` into a [`PinImage`] (same field shape, no shared type by
//! design), calls [`copy_pin`]/[`save_pin`], and mirrors window lifecycle
//! into a [`PinRegistry`]. `flowshot-ui`'s `examples/pin_window.rs` is the
//! reference composition.

mod registry;

pub use registry::{PinRecord, PinRegistry};

use std::path::PathBuf;

use flowshot_core::config::SaveConfig;
use image::{DynamicImage, ImageBuffer, Rgba};

use crate::clipboard::Clipboard;
use crate::error::PinError;
use crate::export::{FileDialogSink, NotifySink};

/// The pixel payload of a pin action: the CURRENT rotated buffer at FULL
/// opacity (Flameshot parity - window opacity is a display effect that
/// never touches copied/saved pixels), in the same shape as the UI's
/// `PinSnapshot` minus the window id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinImage {
    /// Width in physical pixels (post-rotation).
    pub width: u32,
    /// Height in physical pixels (post-rotation).
    pub height: u32,
    /// Upright RGBA8 pixels for the rotated orientation, full opacity.
    pub rgba: Vec<u8>,
}

impl PinImage {
    /// Converts to the image-crate type the clipboard and export seams
    /// consume.
    ///
    /// # Errors
    ///
    /// [`PinError::InvalidBuffer`] when `rgba` is shorter than
    /// `width * height * 4`.
    pub fn to_dynamic(&self) -> Result<DynamicImage, PinError> {
        let expected = usize::try_from(self.width)
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(self.height).unwrap_or(usize::MAX))
            .saturating_mul(4);
        if self.rgba.len() < expected {
            return Err(PinError::InvalidBuffer {
                width: self.width,
                height: self.height,
                expected,
                actual: self.rgba.len(),
            });
        }
        let buffer =
            ImageBuffer::<Rgba<u8>, _>::from_raw(self.width, self.height, self.rgba.clone())
                .ok_or(PinError::InvalidBuffer {
                    width: self.width,
                    height: self.height,
                    expected,
                    actual: self.rgba.len(),
                })?;
        Ok(DynamicImage::ImageRgba8(buffer))
    }
}

/// Copies a pin to the clipboard through the clipboard seam (`image/png`
/// always, `image/jpeg` appended when `[save].clipboard_format = 'jpeg'`).
///
/// # Errors
///
/// [`PinError::InvalidBuffer`] on a malformed payload,
/// [`PinError::Clipboard`] on encode/transport failure.
pub fn copy_pin(
    image: &PinImage,
    config: &SaveConfig,
    clipboard: &Clipboard,
) -> Result<(), PinError> {
    let dynamic = image.to_dynamic()?;
    crate::export::copy_to_clipboard(&dynamic, config, clipboard)?;
    Ok(())
}

/// Saves a pin to disk through the export seam (strftime pattern,
/// sanitization, collision numeration, format/quality per `[save]`).
///
/// # Errors
///
/// [`PinError::InvalidBuffer`] on a malformed payload,
/// [`PinError::Export`] on path/encode/write failure.
pub fn save_pin(
    image: &PinImage,
    config: &SaveConfig,
    dialog: &dyn FileDialogSink,
    notify: &dyn NotifySink,
) -> Result<PathBuf, PinError> {
    let dynamic = image.to_dynamic()?;
    Ok(crate::export::save(&dynamic, config, dialog, notify)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::MockClipboard;
    use crate::export::NullNotifySink;

    struct NoDialog;
    impl FileDialogSink for NoDialog {
        fn pick_save_path(
            &self,
            _default_name: &str,
        ) -> Result<Option<PathBuf>, crate::error::ExportError> {
            Ok(None)
        }
    }

    /// 2x1 red/white RGBA fixture.
    fn fixture() -> PinImage {
        PinImage {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 255, 255, 255, 255],
        }
    }

    #[test]
    fn copy_pin_offers_png_with_the_exact_pixels() {
        // The mock's clone shares one recorder, so a handle survives the
        // wrap into the facade for assertions.
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        let config = SaveConfig::default();
        copy_pin(&fixture(), &config, &clipboard).unwrap_or_else(|e| panic!("{e}"));
        let offer = mock.last_offer().unwrap_or_else(|| panic!("no offer"));
        assert_eq!(offer.entries.len(), 1);
        assert_eq!(offer.entries[0].mime, "image/png");
        assert_eq!(&offer.entries[0].data[..8], b"\x89PNG\r\n\x1a\n");
        let decoded = image::load_from_memory(&offer.entries[0].data)
            .unwrap_or_else(|e| panic!("{e}"))
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn save_pin_writes_the_file_with_collision_numeration() {
        let dir = std::env::temp_dir().join(format!("flowshot-pin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let config = SaveConfig {
            path: dir.to_string_lossy().into_owned(),
            path_fixed: true,
            filename_pattern: "pin-test".to_owned(),
            ..SaveConfig::default()
        };
        let first = save_pin(&fixture(), &config, &NoDialog, &NullNotifySink)
            .unwrap_or_else(|e| panic!("{e}"));
        let second = save_pin(&fixture(), &config, &NoDialog, &NullNotifySink)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("_1"));
        let decoded = image::open(&first)
            .unwrap_or_else(|e| panic!("{e}"))
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_buffer_is_typed() {
        let broken = PinImage {
            width: 4,
            height: 4,
            rgba: vec![0; 10],
        };
        assert!(matches!(
            broken.to_dynamic(),
            Err(PinError::InvalidBuffer {
                width: 4,
                height: 4,
                ..
            })
        ));
    }
}
