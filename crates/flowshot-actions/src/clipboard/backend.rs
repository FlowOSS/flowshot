//! Clipboard backends and route selection.
//!
//! [`WaylandClipboard`] is the production backend: `zwlr_data_control`
//! via `wl-clipboard-rs` (no `wl-copy` shell-out, no GTK).
//! [`MockClipboard`] records offers in memory so the whole
//! crate tests headless (no compositor).
//!
//! [`select_route`] picks between the data-control route and the GNOME
//! keep-alive fallback ([`super::keepalive`]) from a [`probe_data_control`]
//! result; the probe is injectable so QA can mask data-control and force
//! the fallback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use wl_clipboard_rs::copy;
use wl_clipboard_rs::paste;

use super::ClipboardBackend;
use super::offer::ClipboardOffer;
use crate::error::ClipboardError;

/// Production backend: daemon-owned offers over `zwlr_data_control`.
///
/// # Daemon ownership (draft F27)
///
/// [`ClipboardBackend::serve`] runs `wl-clipboard-rs` in background mode
/// with unlimited request serving: the library spawns its serving thread
/// INSIDE THE CALLING PROCESS and holds the data-control offer for as
/// long as that process lives. Called from the daemon, the
/// offer therefore outlives the capturing UI process — the GUI exits,
/// `wl-paste` still gets bytes. Daemon exit destroys the offer
/// (documented behavior; the daemon's "clipboard offer
/// held" persistence reason keeps it alive while offered).
#[derive(Debug, Clone, Copy, Default)]
pub struct WaylandClipboard;

impl WaylandClipboard {
    /// Create the stateless data-control backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ClipboardBackend for WaylandClipboard {
    fn serve(&self, offer: ClipboardOffer) -> Result<(), ClipboardError> {
        let sources = offer
            .entries
            .into_iter()
            .map(|entry| copy::MimeSource {
                source: copy::Source::Bytes(entry.data),
                mime_type: copy::MimeType::Specific(entry.mime),
            })
            .collect();
        let mut options = copy::Options::new();
        // Unlimited + background: the offer is served by a thread in THIS
        // process until another client takes the clipboard (daemon ownership).
        options
            .serve_requests(copy::ServeRequests::Unlimited)
            .foreground(false);
        Ok(copy::copy_multi(options, sources)?)
    }
}

/// In-memory recording backend for headless tests.
///
/// Clone shares one recorder, so a handle stays usable for assertions
/// after the clone is wrapped into a [`super::Clipboard`].
#[derive(Debug, Clone, Default)]
pub struct MockClipboard {
    inner: Arc<MockInner>,
}

#[derive(Debug, Default)]
struct MockInner {
    offers: Mutex<Vec<ClipboardOffer>>,
    failing: AtomicBool,
}

impl MockClipboard {
    /// Create an empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every offer served so far, in order.
    #[must_use]
    pub fn offers(&self) -> Vec<ClipboardOffer> {
        self.inner
            .offers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The most recent offer, if any.
    #[must_use]
    pub fn last_offer(&self) -> Option<ClipboardOffer> {
        self.inner
            .offers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned()
    }

    /// Make subsequent `serve` calls fail (transport-error injection).
    pub fn set_failing(&self, failing: bool) {
        self.inner.failing.store(failing, Ordering::SeqCst);
    }
}

impl ClipboardBackend for MockClipboard {
    fn serve(&self, offer: ClipboardOffer) -> Result<(), ClipboardError> {
        if self.inner.failing.load(Ordering::SeqCst) {
            return Err(ClipboardError::Transport(copy::Error::NoSeats));
        }
        self.inner
            .offers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(offer);
        Ok(())
    }
}

/// Which clipboard mechanism the environment supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRoute {
    /// `zwlr_data_control` available (wlroots/Hyprland/KDE): daemon-owned
    /// offer via [`WaylandClipboard`]. LIVE-verified class.
    DataControl,
    /// Portal-only GNOME without data-control: keep-alive fallback
    /// (`crate::clipboard::keepalive`). UNIT-LEVEL ONLY — live QA
    /// deferred, never claimed verified.
    GnomeKeepAlive,
}

/// Select the clipboard route from a data-control probe result.
///
/// The probe result is injectable so QA can mask data-control and assert
/// the fallback selection.
#[must_use]
pub fn select_route(data_control_available: bool) -> ClipboardRoute {
    if data_control_available {
        tracing::info!(route = "data-control", "clipboard route selected");
        ClipboardRoute::DataControl
    } else {
        tracing::warn!(
            route = "gnome-keep-alive",
            "zwlr_data_control unavailable; selected GNOME keep-alive route"
        );
        ClipboardRoute::GnomeKeepAlive
    }
}

/// Read-only probe: does the compositor speak `zwlr_data_control`?
///
/// Asks the compositor for the current offer's MIME types — a pure read.
/// An empty clipboard still proves the protocol works; a missing protocol
/// (or no compositor at all) means unavailable.
#[must_use]
pub fn probe_data_control() -> bool {
    match paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified) {
        // Protocol round-trip succeeded; clipboard content state (empty,
        // no seats) is irrelevant to availability.
        Ok(_)
        | Err(
            paste::Error::NoSeats
            | paste::Error::ClipboardEmpty
            | paste::Error::NoMimeType
            | paste::Error::SeatNotFound,
        ) => true,
        // No data-control protocol, or no compositor to ask.
        Err(
            paste::Error::MissingProtocol { .. }
            | paste::Error::SocketOpenError(_)
            | paste::Error::WaylandConnection(_)
            | paste::Error::WaylandCommunication(_)
            | paste::Error::PrimarySelectionUnsupported
            | paste::Error::PipeCreation(_),
        ) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::Clipboard;

    #[derive(Clone, Default)]
    struct LogBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for LogBuf {
        type Writer = Self;
        fn make_writer(&self) -> Self {
            self.clone()
        }
    }

    fn log_contents(buf: &LogBuf) -> String {
        String::from_utf8_lossy(&buf.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }

    #[test]
    fn mock_records_offers_in_order() -> Result<(), ClipboardError> {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        clipboard.copy_text("first")?;
        clipboard.copy_text("second")?;
        let offers = mock.offers();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].entries[0].data.as_ref(), b"first");
        assert_eq!(offers[1].entries[0].data.as_ref(), b"second");
        Ok(())
    }

    #[test]
    fn mock_failure_injection_surfaces_transport_error() {
        let mock = MockClipboard::new();
        let clipboard = Clipboard::new(mock.clone());
        mock.set_failing(true);
        let result = clipboard.copy_text("nope");
        assert!(matches!(result, Err(ClipboardError::Transport(_))));
        assert!(mock.offers().is_empty());
    }

    #[test]
    fn data_control_available_selects_data_control_route() {
        let buf = LogBuf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(buf.clone())
            .finish();
        let route = tracing::subscriber::with_default(subscriber, || select_route(true));
        assert_eq!(route, ClipboardRoute::DataControl);
        assert!(log_contents(&buf).contains("data-control"));
    }

    /// QA failure scenario: data-control masked (probe
    /// override) -> keep-alive path selected, asserted via the trace.
    #[test]
    fn masked_data_control_selects_keep_alive_route_with_trace() {
        let buf = LogBuf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(buf.clone())
            .finish();
        let route = tracing::subscriber::with_default(subscriber, || select_route(false));
        assert_eq!(route, ClipboardRoute::GnomeKeepAlive);
        assert!(log_contents(&buf).contains("gnome-keep-alive"));
    }
}
