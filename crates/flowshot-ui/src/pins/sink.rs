//! The pin action seam ("copy/save items wired via
//! action-callback traits").
//!
//! Dependency inversion: the UI crate owns this INTERFACE and never sees
//! the platform action crates (purity gate - `flowshot-actions` is
//! platform-native by design). The binary layer (CLI or daemon)
//! implements [`PinActionSink`] by calling
//! `flowshot_actions::pin::{copy_pin, save_pin}` (the clipboard/export
//! modules) and mirrors window lifecycle into
//! `flowshot_actions::pin::PinRegistry` (the daemon's "pins alive"
//! persistence reason). End-to-end assertion is a binary-layer flow.

/// Host-assigned pin identity (matches `flowshot_actions::pin::PinRegistry`
/// keys by raw value - the crates share no types by design).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PinId(u64);

impl PinId {
    /// Wraps a host-assigned raw id.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw id (the registry bridge value).
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The pixel payload handed to the sink on copy/save: the pin's CURRENT
/// rotated buffer at FULL opacity (Flameshot parity - `copyToClipboard`
/// copies the rotated `m_pixmap`; window opacity is a display effect that
/// never touches the copied pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinSnapshot {
    /// Which pin produced this snapshot.
    pub id: PinId,
    /// Width in physical pixels (post-rotation).
    pub width: u32,
    /// Height in physical pixels (post-rotation).
    pub height: u32,
    /// Upright RGBA8 pixels for the rotated orientation, full opacity.
    pub rgba: Vec<u8>,
}

/// The action-callback seam the binary layer implements (the
/// clipboard/export modules sit behind it). Called on the event-loop thread; implementations
/// should be quick (the clipboard backend serves from its own thread, the
/// export writes one small file).
pub trait PinActionSink: std::fmt::Debug + Send + Sync + 'static {
    /// Copy the snapshot to the clipboard.
    ///
    /// # Errors
    ///
    /// Opaque typed error from the implementing layer; the shell logs it
    /// and keeps the pin alive (a failed copy never closes a pin).
    fn copy(&self, snapshot: PinSnapshot) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Save the snapshot to disk (the implementing layer owns path policy
    /// and dialogs).
    ///
    /// # Errors
    ///
    /// Opaque typed error from the implementing layer; logged, pin stays.
    fn save(&self, snapshot: PinSnapshot) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// A pin window closed (user action, Esc, or compositor): the host
    /// unregisters it from the pin registry ("pins alive" bookkeeping).
    fn pin_closed(&self, id: PinId);
}
