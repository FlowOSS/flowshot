//! GNOME keep-alive clipboard path (portal-only environments WITHOUT
//! `zwlr_data_control`).
//!
//! Clean-room equivalent of Flameshot's `screenshotsaver.cpp` L253-270
//! (`ClipboardWatcherMimeData` + `saveToClipboardGnomeWorkaround`,
//! draft F27 daemon spec): register a LAZY mime offer, notify the owner
//! on FIRST data access (the compositor fetching the bytes is what lets
//! it take over the offer), and force-close after a 500 ms safety window
//! if the compositor never fetches.
//!
//! VERIFICATION CLASS (plan todo 28, Metis #2): this path is UNIT-LEVEL
//! ONLY — live QA deferred, never claimed verified. The data-control
//! route ([`super::backend`]) is the live-verified one.
//!
//! This module is the pure state machine; the runtime that registers the
//! actual lazy offer on GNOME (no GTK allowed, plan todo 28 Must-NOT)
//! wires these states and effects when it lands.

use std::time::Duration;

/// Safety-close delay: force-close the keep-alive offer this long after
/// registration when the compositor never requested the data
/// (Flameshot parity constant, `QTimer::singleShot(500, ...)`).
pub const SAFETY_CLOSE: Duration = Duration::from_millis(500);

/// State of the keep-alive dance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAliveState {
    /// Lazy offer registered, keep-alive surface hidden, safety timer
    /// armed; waiting for the compositor's first data request.
    Offering,
    /// First access served: owner notified, close scheduled. The
    /// compositor now holds (or is fetching) the data.
    Notified,
    /// Safety timer expired with no access: force-closing (the offer is
    /// lost — documented GNOME fallback behavior).
    TimedOut,
    /// Terminal: keep-alive released, process free to exit.
    Closed,
}

/// Input events driving the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAliveEvent {
    /// The compositor (or any paste target) requested the offered data.
    DataRequested,
    /// The [`SAFETY_CLOSE`] timer fired.
    SafetyTimeout,
    /// The keep-alive surface finished closing.
    CloseCompleted,
}

/// Side effects the runtime must perform for a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAliveEffect {
    /// Register the lazy mime offer (data materialized on first fetch).
    RegisterLazyOffer,
    /// Hide the keep-alive surface (it exists only to hold ownership).
    HideKeepAlive,
    /// Arm the [`SAFETY_CLOSE`] timer.
    StartSafetyTimer,
    /// Serve the requested bytes to the compositor.
    ServeData,
    /// Notify the owner exactly once ("capture saved to clipboard").
    NotifyOwner,
    /// Schedule the keep-alive close on the next event-loop turn.
    ScheduleClose,
    /// Log the timeout warning (compositor never fetched).
    WarnTimeout,
    /// Close the keep-alive surface immediately.
    ForceClose,
}

/// The keep-alive state machine driver.
///
/// Create with [`KeepAlive::start`] (which yields the startup effects),
/// then feed events via [`KeepAlive::advance`] and execute the returned
/// effects in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAlive {
    state: KeepAliveState,
    owner_notified: bool,
}

impl KeepAlive {
    /// Begin the keep-alive dance in the [`KeepAliveState::Offering`]
    /// state, returning the startup effects (lazy offer, hidden surface,
    /// armed safety timer).
    #[must_use]
    pub fn start() -> (Self, Vec<KeepAliveEffect>) {
        use KeepAliveEffect::{HideKeepAlive, RegisterLazyOffer, StartSafetyTimer};
        let machine = Self {
            state: KeepAliveState::Offering,
            owner_notified: false,
        };
        (
            machine,
            vec![RegisterLazyOffer, HideKeepAlive, StartSafetyTimer],
        )
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> KeepAliveState {
        self.state
    }

    /// Whether the owner has been notified (notify-on-first-access is
    /// once-only, Flameshot `m_notified` guard).
    #[must_use]
    pub const fn owner_notified(&self) -> bool {
        self.owner_notified
    }

    /// Apply one event, returning the effects the runtime must execute.
    #[must_use]
    pub fn advance(&mut self, event: KeepAliveEvent) -> Vec<KeepAliveEffect> {
        use KeepAliveEffect::{ForceClose, NotifyOwner, ScheduleClose, ServeData, WarnTimeout};
        use KeepAliveEvent::{CloseCompleted, DataRequested, SafetyTimeout};
        use KeepAliveState::{Closed, Notified, Offering, TimedOut};

        match (self.state, event) {
            (Offering, DataRequested) => {
                self.state = Notified;
                self.owner_notified = true;
                vec![ServeData, NotifyOwner, ScheduleClose]
            }
            // Repeat fetch while closing: still serve, but never notify twice.
            (Notified, DataRequested) => vec![ServeData],
            (Offering, SafetyTimeout) => {
                self.state = TimedOut;
                vec![WarnTimeout, ForceClose]
            }
            (Closed, _) | (Notified | TimedOut, SafetyTimeout) | (TimedOut, DataRequested) => {
                Vec::new()
            }
            (_, CloseCompleted) => {
                self.state = Closed;
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeepAliveEffect::{
        ForceClose, HideKeepAlive, NotifyOwner, RegisterLazyOffer, ScheduleClose, ServeData,
        StartSafetyTimer, WarnTimeout,
    };
    use KeepAliveEvent::{CloseCompleted, DataRequested, SafetyTimeout};
    use KeepAliveState::{Closed, Notified, Offering, TimedOut};

    #[test]
    fn start_registers_lazy_offer_hides_surface_and_arms_timer() {
        let (machine, effects) = KeepAlive::start();
        assert_eq!(machine.state(), Offering);
        assert!(!machine.owner_notified());
        assert_eq!(
            effects,
            [RegisterLazyOffer, HideKeepAlive, StartSafetyTimer]
        );
    }

    /// Full transition table (plan todo 28: "state-machine unit-tested").
    #[test]
    fn transition_table() {
        struct Case {
            name: &'static str,
            setup: &'static [KeepAliveEvent],
            event: KeepAliveEvent,
            state: KeepAliveState,
            effects: &'static [KeepAliveEffect],
        }
        let cases = [
            Case {
                name: "first access serves, notifies once, schedules close",
                setup: &[],
                event: DataRequested,
                state: Notified,
                effects: &[ServeData, NotifyOwner, ScheduleClose],
            },
            Case {
                name: "second access serves without re-notifying (m_notified guard)",
                setup: &[DataRequested],
                event: DataRequested,
                state: Notified,
                effects: &[ServeData],
            },
            Case {
                name: "close completes after first access",
                setup: &[DataRequested],
                event: CloseCompleted,
                state: Closed,
                effects: &[],
            },
            Case {
                name: "safety timeout warns and force-closes",
                setup: &[],
                event: SafetyTimeout,
                state: TimedOut,
                effects: &[WarnTimeout, ForceClose],
            },
            Case {
                name: "close completes after timeout",
                setup: &[SafetyTimeout],
                event: CloseCompleted,
                state: Closed,
                effects: &[],
            },
            Case {
                name: "late data request during force-close is ignored",
                setup: &[SafetyTimeout],
                event: DataRequested,
                state: TimedOut,
                effects: &[],
            },
            Case {
                name: "safety timer firing after close was scheduled is a guarded no-op",
                setup: &[DataRequested],
                event: SafetyTimeout,
                state: Notified,
                effects: &[],
            },
            Case {
                name: "external close while offering",
                setup: &[],
                event: CloseCompleted,
                state: Closed,
                effects: &[],
            },
            Case {
                name: "everything after close is ignored",
                setup: &[DataRequested, CloseCompleted],
                event: DataRequested,
                state: Closed,
                effects: &[],
            },
        ];
        for case in cases {
            let (mut machine, _) = KeepAlive::start();
            for event in case.setup {
                let _ = machine.advance(*event);
            }
            let effects = machine.advance(case.event);
            assert_eq!(machine.state(), case.state, "state: {}", case.name);
            assert_eq!(effects, case.effects, "effects: {}", case.name);
        }
    }

    #[test]
    fn owner_notification_is_once_only_across_many_fetches() {
        let (mut machine, _) = KeepAlive::start();
        let mut notify_count = 0;
        for _ in 0..5 {
            notify_count += machine
                .advance(DataRequested)
                .iter()
                .filter(|effect| **effect == NotifyOwner)
                .count();
        }
        assert_eq!(notify_count, 1);
        assert!(machine.owner_notified());
    }

    #[test]
    fn safety_close_is_the_500ms_parity_constant() {
        assert_eq!(SAFETY_CLOSE, Duration::from_millis(500));
    }
}
