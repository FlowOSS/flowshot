//! The daemon's internal command vocabulary and dispatch seam.
//!
//! Bus methods parse their arguments ([`crate::request`]) and hand a typed
//! [`DaemonCommand`] to the configured [`CommandSink`]. The default sink
//! logs (todo 32 scope: the service shell); todo 35's CLI wiring plugs a
//! sink that executes captures, and [`ChannelSink`] streams commands to any
//! consumer task.

use std::fmt;

use tokio::sync::mpsc;

use crate::request::CaptureRequest;

/// One accepted `D-Bus` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonCommand {
    /// `Capture(options)`: interactive region capture with modifiers.
    Capture(CaptureRequest),
    /// `CaptureFull`: full-desktop capture.
    CaptureFull,
    /// `CaptureScreen(n)`: single output by index.
    CaptureScreen(u32),
    /// `Launcher`: open the manual-coordinate launcher dialog (todo 37
    /// surface; dispatched by the todo-33 tray item and CLI `--dialog`).
    Launcher,
    /// `Settings`: open the settings surface (todo 36).
    Settings,
    /// `Invoke(argv)`: a second `flowshot` process forwarded its command
    /// line (single-instance parity UX).
    Invoke(Vec<String>),
}

impl fmt::Display for DaemonCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capture(_) => write!(f, "Capture"),
            Self::CaptureFull => write!(f, "CaptureFull"),
            Self::CaptureScreen(screen) => write!(f, "CaptureScreen({screen})"),
            Self::Launcher => write!(f, "Launcher"),
            Self::Settings => write!(f, "Settings"),
            Self::Invoke(argv) => write!(f, "Invoke({argv:?})"),
        }
    }
}

/// Where accepted commands go. Implementations must be cheap and
/// non-blocking: sinks are called from the bus dispatch tasks.
pub trait CommandSink: Send + Sync + fmt::Debug {
    /// Handle one accepted command.
    fn dispatch(&self, command: DaemonCommand);
}

/// Default sink: one stable structured log line per command (the todo-32
/// acceptance greps `invoke received`).
#[derive(Debug, Clone, Copy)]
pub struct LoggingSink;

impl CommandSink for LoggingSink {
    fn dispatch(&self, command: DaemonCommand) {
        match command {
            DaemonCommand::Invoke(argv) => {
                tracing::info!(argv = ?argv, "invoke received");
            }
            other => tracing::info!(command = %other, "daemon command accepted"),
        }
    }
}

/// Streams commands to a consumer task (the todo-35 execution seam).
#[derive(Debug, Clone)]
pub struct ChannelSink {
    commands: mpsc::UnboundedSender<DaemonCommand>,
}

impl ChannelSink {
    /// A sink forwarding into `commands`.
    #[must_use]
    pub const fn new(commands: mpsc::UnboundedSender<DaemonCommand>) -> Self {
        Self { commands }
    }
}

impl CommandSink for ChannelSink {
    fn dispatch(&self, command: DaemonCommand) {
        if let Err(error) = self.commands.send(command) {
            tracing::warn!(command = %error.0, "command consumer is gone; dropping");
        }
    }
}

/// Records commands in memory (test seam: dispatch captured without a
/// consumer runtime).
#[derive(Debug, Clone, Default)]
pub struct RecordingSink {
    recorded: std::sync::Arc<std::sync::Mutex<Vec<DaemonCommand>>>,
}

impl RecordingSink {
    /// An empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every command dispatched so far, in order.
    #[must_use]
    pub fn commands(&self) -> Vec<DaemonCommand> {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl CommandSink for RecordingSink {
    fn dispatch(&self, command: DaemonCommand) {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_captures_dispatches_in_order() {
        let sink = RecordingSink::new();
        sink.dispatch(DaemonCommand::CaptureFull);
        sink.dispatch(DaemonCommand::Invoke(vec!["capture".to_owned()]));
        assert_eq!(
            sink.commands(),
            vec![
                DaemonCommand::CaptureFull,
                DaemonCommand::Invoke(vec!["capture".to_owned()])
            ]
        );
    }

    #[test]
    fn channel_sink_forwards_to_the_consumer() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = ChannelSink::new(tx);
        sink.dispatch(DaemonCommand::CaptureScreen(2));
        assert_eq!(rx.try_recv(), Ok(DaemonCommand::CaptureScreen(2)));
    }

    #[test]
    fn channel_sink_survives_a_dropped_consumer() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let sink = ChannelSink::new(tx);
        sink.dispatch(DaemonCommand::Settings); // must not panic
    }

    #[test]
    fn display_is_a_stable_token_per_variant() {
        assert_eq!(DaemonCommand::CaptureFull.to_string(), "CaptureFull");
        assert_eq!(
            DaemonCommand::CaptureScreen(3).to_string(),
            "CaptureScreen(3)"
        );
        assert_eq!(DaemonCommand::Launcher.to_string(), "Launcher");
        assert_eq!(DaemonCommand::Settings.to_string(), "Settings");
        assert_eq!(
            DaemonCommand::Invoke(vec!["capture".to_owned()]).to_string(),
            "Invoke([\"capture\"])"
        );
        assert_eq!(
            DaemonCommand::Capture(CaptureRequest::default()).to_string(),
            "Capture"
        );
    }

    #[test]
    fn logging_sink_dispatches_every_variant_without_panic() {
        let sink = LoggingSink;
        for command in [
            DaemonCommand::Capture(CaptureRequest::default()),
            DaemonCommand::CaptureFull,
            DaemonCommand::CaptureScreen(0),
            DaemonCommand::Launcher,
            DaemonCommand::Settings,
            DaemonCommand::Invoke(Vec::new()),
        ] {
            sink.dispatch(command);
        }
    }
}
