//! The startup milestone contract asserted through the emitted events: the
//! documented milestone names in the documented order, the once-only
//! `perf.ui_ready`, and the fact that no child reports readiness before a
//! frame is actually presented.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

use super::StartupClock;

#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuf {
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

fn subscriber(buf: &LogBuf) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(buf.clone())
        .finish()
}

#[test]
fn milestones_emit_in_the_documented_order() {
    let buf = LogBuf::default();
    let mut clock = StartupClock::start();
    tracing::subscriber::with_default(subscriber(&buf), || {
        clock.record("perf.windows_ready");
        clock.record("perf.gpu_instance");
        clock.record("perf.gpu_ready");
        clock.record("perf.renderers_ready");
        clock.report_first_present();
    });

    let log = log_contents(&buf);
    let milestones = [
        "perf.windows_ready",
        "perf.gpu_instance",
        "perf.gpu_ready",
        "perf.renderers_ready",
        "perf.ui_ready",
    ];
    for milestone in milestones {
        assert!(log.contains(milestone), "missing {milestone} in {log}");
    }
    let positions = milestones
        .iter()
        .map(|milestone| log.find(milestone).unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "milestones out of order: {positions:?} in {log}"
    );
}

#[test]
fn ready_milestone_fires_once_across_repeated_frames() {
    let buf = LogBuf::default();
    let mut clock = StartupClock::start();
    tracing::subscriber::with_default(subscriber(&buf), || {
        clock.record("perf.gpu_ready");
        assert!(
            clock.report_first_present(),
            "the first present must report"
        );
        assert!(
            !clock.report_first_present(),
            "later frames must stay quiet"
        );
        assert!(
            !clock.report_first_present(),
            "later frames must stay quiet"
        );
    });

    let log = log_contents(&buf);
    assert_eq!(
        log.matches("perf.ui_ready").count(),
        1,
        "readiness is per-child, not per-frame: {log}"
    );
}

#[test]
fn no_readiness_reported_before_any_present() {
    let buf = LogBuf::default();
    let mut clock = StartupClock::start();
    tracing::subscriber::with_default(subscriber(&buf), || {
        clock.record("perf.windows_ready");
        clock.record("perf.gpu_instance");
        clock.record("perf.gpu_ready");
    });

    let log = log_contents(&buf);
    assert!(
        !log.contains("perf.ui_ready"),
        "a child that never presented must not claim readiness: {log}"
    );
}
