//! Smart daemon lifecycle (Amendment #3 - REPLACES Flameshot's dropped
//! `autoCloseIdleDaemon` flag).
//!
//! Contract: an AUTO-SPAWNED helper daemon exits once it has been idle for
//! the grace period AND no persistence reason holds ([`crate::state`]); a
//! SUPERVISED daemon (`flowshot daemon`, the systemd user unit) ALWAYS
//! persists - init managers own its lifetime. The decision is a pure
//! function ([`LifecyclePolicy::evaluate`]); the [`LifecycleMonitor`] adds
//! the injectable-clock loop around it, so the truth table runs under an
//! accelerated clock with zero real sleeping.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state::{DaemonState, PersistenceReasons};

/// How the daemon process came to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    /// `flowshot daemon` under a supervisor (systemd unit, a terminal, the
    /// todo-35 CLI): always persists; the init system decides when it dies.
    Supervised,
    /// Helper auto-spawned by a client invocation (todo 35 handshake):
    /// exits after the idle grace when no persistence reason holds.
    AutoSpawned,
}

/// The lifecycle decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Keep running; carries the reasons that hold (empty while merely
    /// inside the grace window).
    Persist(PersistenceReasons),
    /// Idle grace elapsed with no persistence reason - shut down.
    Exit,
}

/// Mode + idle grace; the evaluation is pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecyclePolicy {
    /// Supervised or auto-spawned.
    pub mode: DaemonMode,
    /// How long an auto-spawned daemon tolerates zero activity before it
    /// may exit (only consulted in [`DaemonMode::AutoSpawned`]).
    pub idle_grace: Duration,
}

/// Floor/ceiling of the monitor poll interval while reasons hold (the
/// wake-on-change signal makes promptness event-driven; the poll is the
/// safety net).
const HELD_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl LifecyclePolicy {
    /// The pure lifecycle truth function: supervised always persists;
    /// auto-spawned exits iff idle for at least the grace AND no reason
    /// holds. (`as_nanos` comparison: `Duration` ordering operators are not
    /// const on stable.)
    #[must_use]
    pub const fn evaluate(&self, idle_for: Duration, reasons: PersistenceReasons) -> Verdict {
        match self.mode {
            DaemonMode::Supervised => Verdict::Persist(reasons),
            DaemonMode::AutoSpawned => {
                if idle_for.as_nanos() >= self.idle_grace.as_nanos() && !reasons.any() {
                    Verdict::Exit
                } else {
                    Verdict::Persist(reasons)
                }
            }
        }
    }

    /// How long the monitor sleeps between evaluations when nothing wakes
    /// it: the remaining grace, or a fixed 5 s poll interval once the
    /// grace is used up but reasons still hold (prevents a zero-sleep hot
    /// loop).
    #[must_use]
    pub fn wait_hint(&self, idle_for: Duration) -> Duration {
        self.idle_grace
            .checked_sub(idle_for)
            .filter(|remaining| *remaining > Duration::ZERO)
            .unwrap_or(HELD_POLL_INTERVAL)
    }
}

/// Injectable time source: the production clock sleeps on the tokio timer
/// wheel; the test clock advances virtual time instantly (accelerated
/// truth-table runs, no real sleeps).
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The current instant.
    fn now(&self) -> Instant;
    /// Resolves after `duration` has passed (production) or after the
    /// virtual offset advanced by `duration` (tests).
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Production clock: wall-monotonic `Instant` + tokio timer sleeps.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// The idle-exit loop: evaluates the policy against the shared state on a
/// clock-driven cadence, woken early whenever a persistence input changes.
#[derive(Debug)]
pub struct LifecycleMonitor {
    policy: LifecyclePolicy,
    state: Arc<DaemonState>,
    clock: Arc<dyn Clock>,
}

impl LifecycleMonitor {
    /// A monitor for `policy` over `state`, timed by `clock`.
    #[must_use]
    pub fn new(policy: LifecyclePolicy, state: Arc<DaemonState>, clock: Arc<dyn Clock>) -> Self {
        Self {
            policy,
            state,
            clock,
        }
    }

    /// One policy evaluation at the clock's current instant.
    #[must_use]
    pub fn tick(&self) -> Verdict {
        let idle_for = self.state.idle_for(self.clock.now());
        self.policy.evaluate(idle_for, self.state.reasons())
    }

    /// Runs until the policy says [`Verdict::Exit`], returning the idle
    /// duration at exit. In [`DaemonMode::Supervised`] this never returns -
    /// the daemon's shutdown select races it against the signal handler.
    ///
    /// # Cancel safety
    ///
    /// Dropping this future mid-sleep loses nothing: every evaluation
    /// re-reads fresh state.
    pub async fn run_until_exit(&self) -> Duration {
        loop {
            let idle_for = self.state.idle_for(self.clock.now());
            if self.policy.evaluate(idle_for, self.state.reasons()) == Verdict::Exit {
                tracing::info!(
                    idle_secs = idle_for.as_secs(),
                    "idle grace elapsed with no persistence reason - exiting"
                );
                return idle_for;
            }
            let wait = self.policy.wait_hint(idle_for);
            tokio::select! {
                () = self.clock.sleep(wait) => {}
                () = self.state.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Accelerated clock: `sleep` advances the virtual offset at
    /// construction and resolves after one scheduler yield - the truth
    /// table runs in microseconds, and the yield keeps a held-reason loop
    /// fair against the test body on a current-thread runtime.
    #[derive(Debug)]
    struct MockClock {
        base: Instant,
        millis: AtomicU64,
    }

    impl MockClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                millis: AtomicU64::new(0),
            }
        }

        fn advance(&self, duration: Duration) {
            let millis = duration.as_millis().try_into().unwrap_or(u64::MAX);
            self.millis.fetch_add(millis, Ordering::Relaxed);
        }

        fn virtual_now(&self) -> Instant {
            let elapsed = Duration::from_millis(self.millis.load(Ordering::Relaxed));
            self.base.checked_add(elapsed).unwrap_or(self.base)
        }
    }

    impl Clock for MockClock {
        fn now(&self) -> Instant {
            self.virtual_now()
        }

        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.advance(duration);
            Box::pin(async { tokio::task::yield_now().await })
        }
    }

    fn reasons() -> PersistenceReasons {
        PersistenceReasons::default()
    }

    fn auto(grace_secs: u64) -> LifecyclePolicy {
        LifecyclePolicy {
            mode: DaemonMode::AutoSpawned,
            idle_grace: Duration::from_secs(grace_secs),
        }
    }

    // ---- the pure truth table (Amendment #3 smart lifecycle) ----

    #[test]
    fn auto_spawned_exits_only_after_grace_without_reasons() {
        let policy = auto(60);
        // Inside the grace window: persist even with no reasons.
        assert_eq!(
            policy.evaluate(Duration::from_secs(59), reasons()),
            Verdict::Persist(reasons())
        );
        // At/after the grace boundary with no reasons: exit.
        assert_eq!(
            policy.evaluate(Duration::from_secs(60), reasons()),
            Verdict::Exit
        );
        assert_eq!(
            policy.evaluate(Duration::from_secs(61), reasons()),
            Verdict::Exit
        );
    }

    #[test]
    fn each_persistence_reason_holds_exit_past_grace() {
        let policy = auto(60);
        let idle = Duration::from_secs(3600);
        let held = [
            PersistenceReasons {
                tray: true,
                ..reasons()
            },
            PersistenceReasons {
                shortcuts: true,
                ..reasons()
            },
            PersistenceReasons {
                pins_alive: true,
                ..reasons()
            },
            PersistenceReasons {
                clipboard_offer: true,
                ..reasons()
            },
        ];
        for held_reasons in held {
            assert_eq!(
                policy.evaluate(idle, held_reasons),
                Verdict::Persist(held_reasons),
                "reason {:?} must hold exit",
                held_reasons.names()
            );
        }
    }

    #[test]
    fn releasing_each_reason_permits_exit() {
        let policy = auto(60);
        let idle = Duration::from_secs(3600);
        // Every single-reason set released back to the empty set exits.
        assert_eq!(policy.evaluate(idle, reasons()), Verdict::Exit);
        // Two-reason sets need BOTH released.
        let both = PersistenceReasons {
            tray: true,
            pins_alive: true,
            ..reasons()
        };
        assert_eq!(policy.evaluate(idle, both), Verdict::Persist(both));
        let only_tray = PersistenceReasons {
            tray: true,
            ..reasons()
        };
        assert_eq!(
            policy.evaluate(idle, only_tray),
            Verdict::Persist(only_tray)
        );
    }

    #[test]
    fn supervised_never_exits_regardless_of_idle_or_reasons() {
        let policy = LifecyclePolicy {
            mode: DaemonMode::Supervised,
            idle_grace: Duration::from_secs(1),
        };
        let huge_idle = Duration::from_hours(8_760);
        assert_eq!(
            policy.evaluate(huge_idle, reasons()),
            Verdict::Persist(reasons())
        );
        let tray = PersistenceReasons {
            tray: true,
            ..reasons()
        };
        assert_eq!(policy.evaluate(huge_idle, tray), Verdict::Persist(tray));
    }

    #[test]
    fn wait_hint_shrinks_with_idle_and_floors_when_held() {
        let policy = auto(60);
        assert_eq!(policy.wait_hint(Duration::ZERO), Duration::from_secs(60));
        assert_eq!(
            policy.wait_hint(Duration::from_secs(58)),
            Duration::from_secs(2)
        );
        // Grace exhausted but reasons hold: the poll floor, never zero
        // (a zero sleep would hot-loop).
        assert_eq!(
            policy.wait_hint(Duration::from_secs(60)),
            HELD_POLL_INTERVAL
        );
        assert_eq!(
            policy.wait_hint(Duration::from_secs(600)),
            HELD_POLL_INTERVAL
        );
    }

    // ---- the monitor under the accelerated clock ----

    fn monitor(
        policy: LifecyclePolicy,
        tray: bool,
    ) -> (LifecycleMonitor, Arc<MockClock>, Arc<DaemonState>) {
        let clock = Arc::new(MockClock::new());
        let state = Arc::new(DaemonState::new(tray, clock.now()));
        let dyn_clock: Arc<dyn Clock> = clock.clone();
        let monitor = LifecycleMonitor::new(policy, Arc::clone(&state), dyn_clock);
        (monitor, clock, state)
    }

    #[test]
    fn tick_mirrors_the_policy_against_live_state() {
        let (monitor, clock, state) = monitor(auto(60), false);
        assert_eq!(monitor.tick(), Verdict::Persist(reasons()));
        clock.advance(Duration::from_secs(61));
        assert_eq!(monitor.tick(), Verdict::Exit);
        // A pin appearing pulls the verdict back to persist.
        state.register_pin(flowshot_actions::pin::PinRecord {
            id: 1,
            width: 10,
            height: 10,
            opened_at: std::time::SystemTime::UNIX_EPOCH,
        });
        let held = PersistenceReasons {
            pins_alive: true,
            ..reasons()
        };
        assert_eq!(monitor.tick(), Verdict::Persist(held));
        // Closing the pin releases exit again.
        state.unregister_pin(1);
        assert_eq!(monitor.tick(), Verdict::Exit);
    }

    #[tokio::test]
    async fn run_until_exit_returns_after_the_virtual_grace() {
        let (monitor, _clock, _state) = monitor(auto(1), false);
        // MockClock sleeps advance virtual time instantly: the loop
        // converges in ~1 virtual second of iterations without real sleep.
        let idle = tokio::time::timeout(Duration::from_secs(10), monitor.run_until_exit())
            .await
            .unwrap_or_else(|_| panic!("monitor did not exit on the accelerated clock"));
        assert!(idle >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn run_until_exit_waits_out_a_held_reason_then_exits_on_release() {
        let (monitor, _clock, state) = monitor(auto(1), true); // tray holds
        let runner = tokio::spawn(async move { monitor.run_until_exit().await });
        // Yield so the runner reaches its held-reason loop (its mock sleep
        // yields back every iteration, so the two tasks interleave fairly).
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!runner.is_finished());
        // Releasing the reason must let the (already idle) monitor exit.
        state.set_tray(false);
        assert!(
            tokio::time::timeout(Duration::from_secs(10), runner)
                .await
                .is_ok()
        );
    }

    #[test]
    fn mock_clock_sleep_advances_virtual_time() {
        let clock = MockClock::new();
        let t0 = clock.now();
        // The mock advances eagerly at construction, so even an unpolled
        // sleep moves virtual time (deterministic truth-table runs).
        let sleep = clock.sleep(Duration::from_secs(42));
        drop(sleep);
        assert_eq!(clock.now(), t0 + Duration::from_secs(42));
    }
}
