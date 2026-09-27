//! Shared daemon state: the persistence-reason inputs of the smart
//! lifecycle (Amendment #3) plus last-activity tracking.
//!
//! The four persistence reasons (plan todo 32):
//!
//! 1. tray enabled - `[daemon].tray` at startup, runtime-updatable by the
//!    todo-33 tray module;
//! 2. global shortcuts registered - the todo-34 portal triggers need a
//!    resident daemon, so registration pins the process;
//! 3. pins alive - the todo-30 [`PinRegistry`] hosted by this daemon;
//! 4. clipboard offer held - the todo-28 daemon-owned data-control offer
//!    dies with the process, so holding one pins it.
//!
//! Clipboard-release detection caveat: `wl-clipboard-rs` gives no
//! "selection replaced" callback, so the offer flag is set when the daemon
//! serves a capture copy and cleared through [`DaemonState::set_clipboard_offer_held`]
//! by whoever observes the release (todo 35/38 wiring); recorded in the
//! notepad.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flowshot_actions::pin::{PinRecord, PinRegistry};
use tokio::sync::Notify;

/// The persistence-reason snapshot the lifecycle policy consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four independent Amendment-#3 persistence reasons ARE the truth table"
)]
pub struct PersistenceReasons {
    /// `[daemon].tray` (or a runtime tray registration, todo 33).
    pub tray: bool,
    /// Global shortcuts registered through the portal (todo 34).
    pub shortcuts: bool,
    /// At least one pin window is alive (todo 30 registry).
    pub pins_alive: bool,
    /// A daemon-owned clipboard offer is held (todo 28).
    pub clipboard_offer: bool,
}

impl PersistenceReasons {
    /// Whether any reason holds - the auto-spawned daemon persists while
    /// this is `true`.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.tray || self.shortcuts || self.pins_alive || self.clipboard_offer
    }

    /// The held reasons as stable log tokens, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::with_capacity(4);
        if self.tray {
            names.push("tray");
        }
        if self.shortcuts {
            names.push("shortcuts");
        }
        if self.pins_alive {
            names.push("pins");
        }
        if self.clipboard_offer {
            names.push("clipboard-offer");
        }
        names
    }
}

/// Live daemon state shared between the bus interface, the lifecycle
/// monitor, and (later) the tray/shortcut/pin hosts.
///
/// Flag ordering: stores are `Release` and loads `Acquire`, paired with the
/// [`Notify`]-based wake below - a reason cleared before `notify_one` is
/// visible to the woken monitor. The pure-poll path ([`Self::reasons`])
/// tolerates eventual visibility: lifecycle decisions are on a >=100 ms
/// timescale.
#[derive(Debug)]
pub struct DaemonState {
    tray: AtomicBool,
    shortcuts: AtomicBool,
    clipboard_offer: AtomicBool,
    pins: Mutex<PinRegistry>,
    last_capture: Mutex<Option<flowshot_ui::ExportedImage>>,
    last_activity: Mutex<Instant>,
    wake: Notify,
}

impl DaemonState {
    /// State seeded from config: `tray_enabled` is `[daemon].tray`, and
    /// `now` is the initial last-activity stamp (inject the clock's `now`
    /// for deterministic tests).
    #[must_use]
    pub fn new(tray_enabled: bool, now: Instant) -> Self {
        Self {
            tray: AtomicBool::new(tray_enabled),
            shortcuts: AtomicBool::new(false),
            clipboard_offer: AtomicBool::new(false),
            pins: Mutex::new(PinRegistry::new()),
            last_capture: Mutex::new(None),
            last_activity: Mutex::new(now),
            wake: Notify::new(),
        }
    }

    /// Records activity at `now` (every accepted `D-Bus` call touches this)
    /// and wakes the lifecycle monitor.
    pub fn touch(&self, now: Instant) {
        *self.lock_last_activity() = now;
        self.wake.notify_one();
    }

    /// How long no activity was recorded, relative to `now` (saturating:
    /// a clock skew backwards reads as zero idle, never a panic).
    #[must_use]
    pub fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*self.lock_last_activity())
    }

    /// Sets the tray persistence reason (`[daemon].tray` at startup, the
    /// todo-33 tray module at runtime).
    pub fn set_tray(&self, enabled: bool) {
        self.tray.store(enabled, Ordering::Release);
        self.wake.notify_one();
    }

    /// Sets the global-shortcuts persistence reason (todo 34).
    pub fn set_shortcuts_registered(&self, registered: bool) {
        self.shortcuts.store(registered, Ordering::Release);
        self.wake.notify_one();
    }

    /// Sets the clipboard-offer persistence reason (todo 28 flag; see the
    /// module docs for the release-detection caveat).
    pub fn set_clipboard_offer_held(&self, held: bool) {
        self.clipboard_offer.store(held, Ordering::Release);
        self.wake.notify_one();
    }

    /// Tracks a live pin (todo 30 host bridge).
    pub fn register_pin(&self, record: PinRecord) {
        self.lock_pins().register(record);
        self.wake.notify_one();
    }

    /// Stops tracking a closed pin; returns its record when it was live.
    pub fn unregister_pin(&self, id: u64) -> Option<PinRecord> {
        let removed = self.lock_pins().unregister(id);
        self.wake.notify_one();
        removed
    }

    /// Every live pin id, ascending (deterministic for logs and reporting).
    #[must_use]
    pub fn pin_ids(&self) -> Vec<u64> {
        self.lock_pins().ids()
    }

    /// Remembers the most recent completed capture (todo 38: `flowshot
    /// pin` with no FILE pins the last capture - the daemon-resident
    /// in-memory history slot; ONE image, replaced on every completion).
    pub fn set_last_capture(&self, image: flowshot_ui::ExportedImage) {
        *self.lock_last_capture() = Some(image);
    }

    /// The remembered last capture, when this daemon served one.
    #[must_use]
    pub fn last_capture(&self) -> Option<flowshot_ui::ExportedImage> {
        self.lock_last_capture().clone()
    }

    fn lock_last_capture(&self) -> MutexGuard<'_, Option<flowshot_ui::ExportedImage>> {
        self.last_capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The current persistence-reason snapshot.
    #[must_use]
    pub fn reasons(&self) -> PersistenceReasons {
        PersistenceReasons {
            tray: self.tray.load(Ordering::Acquire),
            shortcuts: self.shortcuts.load(Ordering::Acquire),
            pins_alive: !self.lock_pins().is_empty(),
            clipboard_offer: self.clipboard_offer.load(Ordering::Acquire),
        }
    }

    /// Woken when any persistence input changes (reason released, activity
    /// touched) so the monitor re-evaluates without waiting out its poll
    /// interval. One permit coalesces bursts - the monitor re-reads fresh
    /// state, so coalescing is lossless.
    pub async fn changed(&self) {
        self.wake.notified().await;
    }

    fn lock_pins(&self) -> MutexGuard<'_, PinRegistry> {
        self.pins.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_last_activity(&self) -> MutexGuard<'_, Instant> {
        self.last_activity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u64) -> PinRecord {
        PinRecord {
            id,
            width: 10,
            height: 10,
            opened_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn reasons_snapshot_tracks_every_input() {
        let now = Instant::now();
        let state = DaemonState::new(false, now);
        assert_eq!(state.reasons(), PersistenceReasons::default());
        assert!(!state.reasons().any());

        state.set_tray(true);
        assert!(state.reasons().tray);
        state.set_shortcuts_registered(true);
        assert!(state.reasons().shortcuts);
        state.set_clipboard_offer_held(true);
        assert!(state.reasons().clipboard_offer);
        state.register_pin(record(7));
        assert!(state.reasons().pins_alive);
        assert_eq!(state.pin_ids(), vec![7]);
        assert_eq!(
            state.reasons().names(),
            vec!["tray", "shortcuts", "pins", "clipboard-offer"]
        );

        state.unregister_pin(7);
        state.set_tray(false);
        state.set_shortcuts_registered(false);
        state.set_clipboard_offer_held(false);
        assert!(!state.reasons().any());
        assert!(state.reasons().names().is_empty());
    }

    #[test]
    fn idle_grows_from_touch_and_saturates() {
        let t0 = Instant::now();
        let state = DaemonState::new(false, t0);
        assert_eq!(state.idle_for(t0), Duration::ZERO);
        let t1 = t0 + Duration::from_secs(30);
        assert_eq!(state.idle_for(t1), Duration::from_secs(30));
        state.touch(t1);
        assert_eq!(state.idle_for(t1), Duration::ZERO);
        // A stamp in the future (clock skew) saturates to zero idle.
        assert_eq!(state.idle_for(t0), Duration::ZERO);
    }

    #[tokio::test]
    async fn changing_a_reason_wakes_changed() {
        let state = std::sync::Arc::new(DaemonState::new(false, Instant::now()));
        let waiter = tokio::spawn({
            let state = std::sync::Arc::clone(&state);
            async move { state.changed().await }
        });
        // Yield so the waiter registers before the notify.
        tokio::task::yield_now().await;
        state.set_shortcuts_registered(true);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), waiter)
                .await
                .is_ok()
        );
    }
}
