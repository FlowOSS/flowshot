//! Display-server clipboards with daemon-owned offers.
//!
//! # Ownership model (draft F27)
//!
//! The DAEMON process owns the clipboard offer: [`Clipboard`] hands the
//! offer to a [`ClipboardBackend`], and the production backends
//! ([`WaylandClipboard`], [`X11Clipboard`]) serve it from a thread
//! inside the calling process (Wayland: `zwlr_data_control` via
//! `wl-clipboard-rs` — no `wl-copy` shell-out, no GTK/arboard; X11:
//! ICCCM `CLIPBOARD` selection ownership via `x11rb`). The capturing UI
//! exits freely; the offer lives as long as the daemon, and dies with it
//! (documented behavior). The daemon's lifecycle treats a held offer
//! as a persistence reason.
//!
//! # Session selection
//!
//! [`Clipboard::for_session`] picks the backend from the environment:
//! `WAYLAND_DISPLAY` set → Wayland; else `DISPLAY` set → X11; else
//! [`ClipboardError::NoSession`]. Phase A semantics: on X11 only the
//! headless capture path (capture + post-capture actions with the
//! daemon-owned offer) is supported; the interactive overlay, editor,
//! pins, and dialogs remain Wayland-gated (`flowshot-ui`'s
//! `require_display_server`).
//!
//! # Routes
//!
//! [`probe_data_control`] + [`select_route`] pick between:
//!
//! - [`ClipboardRoute::DataControl`] — wlroots/Hyprland/KDE; the
//!   LIVE-verified class.
//! - [`ClipboardRoute::GnomeKeepAlive`] — portal-only GNOME without
//!   data-control; the `keepalive` state machine (lazy offer,
//!   notify-on-first-access, 500 ms safety close). UNIT-LEVEL ONLY,
//!   live QA deferred.
//!
//! # MIME policy
//!
//! - image copies offer `image/png` ALWAYS, plus `image/jpeg` when
//!   `[save].clipboard_format = 'jpeg'`;
//! - `copy-path` offers `text/plain` (bare path) + `text/uri-list`
//!   (`file:` URI); when an image was copied in the same action run the
//!   uri-list is appended to a combined image+path offer;
//! - plain text offers `text/plain` (the backend auto-adds the common
//!   text variants).

// allow: SIZE_OK — one cohesive clipboard facade: the ownership-model
// contract docs, the backend seam, the session routing, and the facade
// constructors share one MIME policy; splitting scatters the routing rule
// from the constructors that embody it. Marginal overshoot (253 pure LOC);
// first growth extracts the session routing (`SessionKind`/`detect_session`)
// into `clipboard/session.rs`.

mod actions;
mod backend;
mod keepalive;
mod offer;
mod pipeline;
mod x11;

pub use actions::{Action, LegacyFlags, effective_actions, execution_order};
pub use backend::{
    ClipboardRoute, MockClipboard, WaylandClipboard, probe_data_control, select_route,
};
pub use keepalive::{KeepAlive, KeepAliveEffect, KeepAliveEvent, KeepAliveState, SAFETY_CLOSE};
pub use offer::{
    ClipboardOffer, MIME_JPEG, MIME_PNG, MIME_TEXT_PLAIN, MIME_URI_LIST, OfferEntry, file_uri,
    image_and_path_offer, image_mime, image_offer, path_offer,
};
pub use pipeline::{ActionOutcome, PostCapture, PostCaptureReport, run_post_capture};
pub use x11::X11Clipboard;

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use flowshot_core::config::{ClipboardFormat, SaveConfig};
use image::DynamicImage;

use crate::error::ClipboardError;
use crate::export::{encode_jpeg, encode_png};

/// Backend seam: where clipboard offers are actually served.
///
/// Production uses [`WaylandClipboard`] or [`X11Clipboard`] (per
/// session); tests use [`MockClipboard`] (headless, no display server).
pub trait ClipboardBackend: Send + Sync + std::fmt::Debug {
    /// Take clipboard ownership and serve `offer` to paste requests.
    ///
    /// # Errors
    ///
    /// Transport-level failures (no compositor, missing protocol, no
    /// seats).
    fn serve(&self, offer: ClipboardOffer) -> Result<(), ClipboardError>;
}

/// Callback fired when a served clipboard offer is LOST: another client
/// superseded it (X11: the serving thread observes `SelectionClear`) or
/// the serving connection died.
///
/// # Platform asymmetry (documented, deliberate)
///
/// Only [`X11Clipboard`] fires this hook, and only while the exiting
/// serve was still the process's most recent ownership claim (a
/// self-supersede — a second `serve()` — stays silent; see its module
/// docs). The Wayland backend NEVER fires it: `wl-clipboard-rs` offers no
/// "selection replaced" callback, so Wayland callers keep the
/// conservative never-clear behavior (a daemon's held-offer persistence
/// reason is released only by process exit). This is the wired form of
/// the release-detection caveat the `flowshot-daemon` state module
/// documents.
pub type OfferLossHook = Arc<dyn Fn() + Send + Sync>;

/// The display-server session a clipboard backend is picked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// A Wayland session (`WAYLAND_DISPLAY` set).
    Wayland,
    /// An X11 session (`DISPLAY` set, no `WAYLAND_DISPLAY`).
    X11,
}

/// The session routing rule with the environment values injectable
/// (headless tests). Empty values count as unset. `WAYLAND_DISPLAY`
/// wins: an `XWayland` session exports both.
fn session_from_env(
    wayland_display: Option<&OsStr>,
    display: Option<&OsStr>,
) -> Result<SessionKind, ClipboardError> {
    let set = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
    if set(wayland_display) {
        Ok(SessionKind::Wayland)
    } else if set(display) {
        Ok(SessionKind::X11)
    } else {
        Err(ClipboardError::NoSession)
    }
}

/// Detect the display-server session from the environment.
///
/// # Errors
///
/// [`ClipboardError::NoSession`] when neither `WAYLAND_DISPLAY` nor
/// `DISPLAY` is set (or both are empty).
pub fn detect_session() -> Result<SessionKind, ClipboardError> {
    session_from_env(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("DISPLAY").as_deref(),
    )
}

/// Clipboard facade used by the post-capture pipeline.
///
/// Construct via [`Clipboard::wayland`] (daemon production route) or
/// [`Clipboard::new`] with any backend (tests).
#[derive(Debug)]
pub struct Clipboard {
    backend: Box<dyn ClipboardBackend>,
}

impl Clipboard {
    /// Wrap any backend implementation.
    #[must_use]
    pub fn new(backend: impl ClipboardBackend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    /// The data-control backend, for the daemon process (the offer is
    /// owned by THIS process's serving thread — see [`WaylandClipboard`]).
    #[must_use]
    pub fn wayland() -> Self {
        Self::new(WaylandClipboard::new())
    }

    /// The X11 selection-owner backend, for the daemon process (the
    /// offer is owned by THIS process's serving thread — see
    /// [`X11Clipboard`]).
    #[must_use]
    pub fn x11() -> Self {
        Self::new(X11Clipboard::new())
    }

    /// The backend for the current session: `WAYLAND_DISPLAY` set →
    /// [`Self::wayland`]; else `DISPLAY` set → [`Self::x11`].
    ///
    /// # Phase A semantics
    ///
    /// X11 support covers the headless capture path only: capture plus
    /// the post-capture actions, with the daemon owning the clipboard
    /// offer exactly like on Wayland. The interactive overlay, editor,
    /// pins, and dialogs remain Wayland-gated in Phase A
    /// (`flowshot-ui`'s `require_display_server`).
    ///
    /// # Errors
    ///
    /// [`ClipboardError::NoSession`] when neither `WAYLAND_DISPLAY` nor
    /// `DISPLAY` is set.
    pub fn for_session() -> Result<Self, ClipboardError> {
        Ok(match detect_session()? {
            SessionKind::Wayland => Self::wayland(),
            SessionKind::X11 => Self::x11(),
        })
    }

    /// [`Self::for_session`] with an offer-loss hook wired into the X11
    /// backend: the daemon's release path for the `clipboard-offer`
    /// persistence reason when another client supersedes the offer
    /// (`SelectionClear` is observable on X11).
    ///
    /// The Wayland arm DISCARDS the hook (it can never fire — see
    /// [`OfferLossHook`] for the documented asymmetry); callers must not
    /// rely on it for Wayland release detection.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::NoSession`] when neither `WAYLAND_DISPLAY` nor
    /// `DISPLAY` is set.
    pub fn for_session_with_loss_hook(hook: OfferLossHook) -> Result<Self, ClipboardError> {
        Ok(match detect_session()? {
            SessionKind::Wayland => Self::wayland(),
            SessionKind::X11 => Self::new(X11Clipboard::with_loss_hook(hook)),
        })
    }

    /// Copy pre-encoded image bytes under the format's MIME type
    /// (`image/png` or `image/jpeg`).
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Transport`] when the backend cannot take the
    /// clipboard.
    pub fn copy_image(&self, data: &[u8], format: ClipboardFormat) -> Result<(), ClipboardError> {
        let offer = ClipboardOffer {
            entries: vec![OfferEntry::new(image_mime(format), data)],
        };
        self.backend.serve(offer)
    }

    /// Copy text as `text/plain` (the backend auto-adds the common text
    /// variants: `text/plain;charset=utf-8`, `STRING`, `UTF8_STRING`,
    /// `TEXT`).
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Transport`] when the backend cannot take the
    /// clipboard.
    pub fn copy_text(&self, text: &str) -> Result<(), ClipboardError> {
        let offer = ClipboardOffer {
            entries: vec![OfferEntry::new(MIME_TEXT_PLAIN, text.as_bytes())],
        };
        self.backend.serve(offer)
    }

    /// Copy a file path: `text/plain` (bare path) + `text/uri-list`
    /// (`file:` URI). Relative paths are made absolute against the
    /// current directory first.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Transport`] when the backend cannot take the
    /// clipboard.
    pub fn copy_file_path(&self, path: &Path) -> Result<(), ClipboardError> {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.backend.serve(offer::path_offer(&absolute))
    }

    /// Encode and copy a capture per `[save]` config: `image/png`
    /// always, `image/jpeg` appended when `clipboard_format = 'jpeg'`
    /// (encoded at `jpeg_quality`).
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Encode`] when image encoding fails,
    /// [`ClipboardError::Transport`] when the backend cannot take the
    /// clipboard.
    pub fn copy_capture(
        &self,
        image: &DynamicImage,
        config: &SaveConfig,
    ) -> Result<(), ClipboardError> {
        let (png, jpeg) = encode_for_clipboard(image, config)?;
        self.backend.serve(offer::image_offer(png, jpeg))
    }

    /// Combined image + path offer: [`Self::copy_capture`] entries with
    /// `text/plain` + `text/uri-list` for `path` appended (the
    /// uri-list-appended rule for `copy-path` after a saved copy).
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Encode`] when image encoding fails,
    /// [`ClipboardError::Transport`] when the backend cannot take the
    /// clipboard.
    pub fn copy_capture_and_path(
        &self,
        image: &DynamicImage,
        config: &SaveConfig,
        path: &Path,
    ) -> Result<(), ClipboardError> {
        let (png, jpeg) = encode_for_clipboard(image, config)?;
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.backend
            .serve(offer::image_and_path_offer(png, jpeg, &absolute))
    }
}

/// PNG always; JPEG additionally when `[save].clipboard_format` selects
/// it.
fn encode_for_clipboard(
    image: &DynamicImage,
    config: &SaveConfig,
) -> Result<(Vec<u8>, Option<Vec<u8>>), ClipboardError> {
    let png = encode_png(image)?;
    let jpeg = match config.clipboard_format {
        ClipboardFormat::Png => None,
        ClipboardFormat::Jpeg => Some(encode_jpeg(image, config.jpeg_quality)?),
    };
    Ok((png, jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];
    const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

    fn config(format: ClipboardFormat) -> SaveConfig {
        SaveConfig {
            clipboard_format: format,
            ..SaveConfig::default()
        }
    }

    fn mimes(offer: &ClipboardOffer) -> Vec<&str> {
        offer.entries.iter().map(|e| e.mime.as_str()).collect()
    }

    #[test]
    fn copy_image_selects_mime_from_format() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_image(&[1, 2, 3], ClipboardFormat::Png)?;
        clipboard.copy_image(&[4, 5], ClipboardFormat::Jpeg)?;
        let offers = mock.offers();
        assert_eq!(mimes(&offers[0]), [MIME_PNG]);
        assert_eq!(offers[0].entries[0].data.as_ref(), &[1, 2, 3]);
        assert_eq!(mimes(&offers[1]), [MIME_JPEG]);
        assert_eq!(offers[1].entries[0].data.as_ref(), &[4, 5]);
        Ok(())
    }

    #[test]
    fn copy_text_offers_text_plain_bytes() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_text("hello clipboard")?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(mimes(&offer), [MIME_TEXT_PLAIN]);
        assert_eq!(offer.entries[0].data.as_ref(), b"hello clipboard");
        Ok(())
    }

    #[test]
    fn copy_file_path_offers_plain_and_uri_list() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_file_path(Path::new("/tmp/flowshot shot.png"))?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(mimes(&offer), [MIME_TEXT_PLAIN, MIME_URI_LIST]);
        assert_eq!(offer.entries[0].data.as_ref(), b"/tmp/flowshot shot.png");
        assert_eq!(
            offer.entries[1].data.as_ref(),
            b"file:///tmp/flowshot%20shot.png\r\n"
        );
        Ok(())
    }

    #[test]
    fn copy_file_path_absolutizes_relative_paths() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_file_path(Path::new("shots/x.png"))?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(mimes(&offer), [MIME_TEXT_PLAIN, MIME_URI_LIST]);
        let uri = String::from_utf8_lossy(&offer.entries[1].data);
        assert!(uri.starts_with("file:///"), "uri was: {uri}");
        assert!(uri.ends_with("shots/x.png\r\n"), "uri was: {uri}");
        Ok(())
    }

    #[test]
    fn copy_capture_encodes_png_by_default() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_capture(&DynamicImage::new_rgb8(3, 2), &config(ClipboardFormat::Png))?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(mimes(&offer), [MIME_PNG]);
        assert!(offer.entries[0].data.starts_with(&PNG_MAGIC));
        Ok(())
    }

    #[test]
    fn copy_capture_appends_jpeg_when_configured() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_capture(
            &DynamicImage::new_rgb8(3, 2),
            &config(ClipboardFormat::Jpeg),
        )?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(mimes(&offer), [MIME_PNG, MIME_JPEG]);
        assert!(offer.entries[0].data.starts_with(&PNG_MAGIC));
        assert!(offer.entries[1].data.starts_with(&JPEG_MAGIC));
        Ok(())
    }

    #[test]
    fn copy_capture_and_path_builds_combined_offer() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_capture_and_path(
            &DynamicImage::new_rgb8(3, 2),
            &config(ClipboardFormat::Jpeg),
            Path::new("/tmp/s.png"),
        )?;
        let offer = mock.last_offer().unwrap_or_default();
        assert_eq!(
            mimes(&offer),
            [MIME_PNG, MIME_JPEG, MIME_TEXT_PLAIN, MIME_URI_LIST]
        );
        assert!(offer.entries[0].data.starts_with(&PNG_MAGIC));
        assert!(offer.entries[1].data.starts_with(&JPEG_MAGIC));
        assert_eq!(offer.entries[2].data.as_ref(), b"/tmp/s.png");
        assert_eq!(offer.entries[3].data.as_ref(), b"file:///tmp/s.png\r\n");
        Ok(())
    }

    #[test]
    fn backend_transport_failure_propagates() {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        mock.set_failing(true);
        let result = clipboard.copy_capture(&DynamicImage::new_rgb8(2, 2), &SaveConfig::default());
        assert!(matches!(result, Err(ClipboardError::Transport(_))));
    }

    #[test]
    fn session_prefers_wayland_when_both_env_vars_set() {
        // An XWayland session exports both; the native session wins.
        let kind = session_from_env(Some(OsStr::new("wayland-0")), Some(OsStr::new(":0")));
        assert!(matches!(kind, Ok(SessionKind::Wayland)));
    }

    #[test]
    fn session_falls_back_to_x11_without_wayland_display() {
        let kind = session_from_env(None, Some(OsStr::new(":0")));
        assert!(matches!(kind, Ok(SessionKind::X11)));
    }

    #[test]
    fn session_without_either_env_var_is_no_session_error() {
        let result = session_from_env(None, None);
        assert!(matches!(result, Err(ClipboardError::NoSession)));
        let message = ClipboardError::NoSession.to_string();
        assert!(
            message.contains("WAYLAND_DISPLAY") && message.contains("DISPLAY"),
            "error must name both session variables: {message}"
        );
    }

    #[test]
    fn session_treats_empty_env_values_as_unset() {
        let result = session_from_env(Some(OsStr::new("")), Some(OsStr::new("")));
        assert!(matches!(result, Err(ClipboardError::NoSession)));
    }
}
