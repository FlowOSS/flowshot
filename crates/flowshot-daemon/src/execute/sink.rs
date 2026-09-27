//! The daemon-side execution seams (todo 38): the [`ExecutingSink`] that
//! replaces the todo-32 `LoggingSink` default, the window-session
//! [`Heartbeat`] that keeps an auto-spawned daemon alive while a session
//! child is open, and the `flowshot pin` last-capture resolution.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{ExecCtx, ExecOutcome, ExecuteError, pin};
use crate::command::{CommandSink, DaemonCommand, ExecutionOutcome, ExecutionReceipt};

/// The heartbeat interval: comfortably inside the smallest sane idle
/// grace so an open window session never idles the daemon out.
const HEARTBEAT: Duration = Duration::from_secs(20);

/// Keeps the auto-spawned daemon's idle clock fresh while a window
/// session (overlay / launcher / settings) is open; dropping it stops the
/// beat PROMPTLY (the worker parks on an interruptible receive - a
/// non-interruptible sleep made `drop` an up-to-20 s blocking join that
/// stalled the executor's single-thread runtime, delaying every session
/// result - including the failure receipts the bus caller waits on). The
/// four Amendment-#3 persistence reasons stay untouched - an in-flight
/// session is transient ACTIVITY (touch), not a residency reason. No-op in
/// the one-shot CLI process (no daemon state).
#[derive(Debug)]
pub struct Heartbeat {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Heartbeat {
    /// Starts the beat for a daemon-resident context.
    #[must_use]
    pub fn start(ctx: &ExecCtx) -> Self {
        let Some(state) = ctx.state.clone() else {
            return Self {
                stop: None,
                worker: None,
            };
        };
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("flowshot-heartbeat".to_owned())
            .spawn(move || {
                state.touch(Instant::now());
                while stopped.recv_timeout(HEARTBEAT).is_err() {
                    state.touch(Instant::now());
                }
            })
            .ok();
        Self {
            stop: Some(stop),
            worker,
        }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// `flowshot pin [FILE]`: pins an image file, or this daemon's last
/// completed capture (the in-memory history slot in [`DaemonState`]).
pub(super) async fn pin_last_or_file(
    file: Option<PathBuf>,
    ctx: &ExecCtx,
) -> Result<ExecOutcome, ExecuteError> {
    let (config, _) = ctx.load_config();
    let image = match file {
        Some(path) => {
            let loaded = image::open(&path)
                .map_err(|error| {
                    ExecuteError::Usage(format!(
                        "{} is not a readable image: {error}",
                        path.display()
                    ))
                })?
                .to_rgba8();
            let (width, height) = loaded.dimensions();
            flowshot_ui::ExportedImage {
                width,
                height,
                rgba: loaded.into_raw(),
            }
        }
        None => ctx
            .state
            .as_ref()
            .and_then(|state| state.last_capture())
            .ok_or_else(|| {
                ExecuteError::Usage(
                    "this daemon has no previous capture to pin; pass an image FILE".to_owned(),
                )
            })?,
    };
    pin::spawn_pin(&image, &config, ctx).await?;
    Ok(ExecOutcome::Done(
        flowshot_actions::clipboard::PostCaptureReport::default(),
    ))
}

/// The daemon's executing sink (todo 32's `CommandSink` seam): every
/// command runs on a dedicated thread with a current-thread tokio
/// runtime, so window sessions block their own thread, the bus dispatch
/// task never blocks, and the `zbus` async-io reactor stays untouched.
#[derive(Debug, Clone)]
pub struct ExecutingSink {
    ctx: ExecCtx,
}

/// The startup reply window (the silent-failure fix): how long a
/// bus-forwarded command watches for an early execution failure before
/// replying "accepted" and letting the execution run in the background
/// (the fire-and-forget UX for window sessions and delayed captures).
/// Sized to cover a session child's startup death - process spawn, runtime
/// init, and capture-ladder failure - with margin on debug builds;
/// failures AFTER the window stay daemon-log-only (recorded limitation,
/// issues.md).
const STARTUP_REPLY_WINDOW: Duration = Duration::from_secs(5);

impl ExecutingSink {
    /// A sink executing against `ctx`.
    #[must_use]
    pub const fn new(ctx: ExecCtx) -> Self {
        Self { ctx }
    }
}

impl CommandSink for ExecutingSink {
    fn dispatch(&self, command: DaemonCommand) {
        // Untracked callers (tray, shortcuts): nobody awaits the receipt.
        let _ = self.dispatch_tracked(command);
    }

    fn dispatch_tracked(&self, command: DaemonCommand) -> Option<ExecutionReceipt> {
        let ctx = self.ctx.clone();
        let (reply, receipt) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name(format!("flowshot-exec-{command}"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "executor runtime build failed");
                        let _ = reply.send(Err(error.to_string()));
                        return;
                    }
                };
                let mut execution = Box::pin(super::execute(command, &ctx));
                // The timeout future binds to the runtime's timer driver
                // at CONSTRUCTION: build it inside block_on, not before.
                let watched = runtime.block_on(async {
                    tokio::time::timeout(STARTUP_REPLY_WINDOW, &mut execution).await
                });
                match watched {
                    Ok(result) => {
                        let _ = reply.send(report(result));
                    }
                    Err(_window_elapsed) => {
                        let _ = reply.send(Ok(()));
                        let _ = report(runtime.block_on(execution));
                    }
                }
            });
        if let Err(error) = spawned {
            // The closure (holding `reply`) was dropped: the bus side sees
            // the dead receipt and surfaces it as a typed failure.
            tracing::error!(%error, "executor thread spawn failed");
        }
        Some(receipt)
    }
}

fn report(result: Result<ExecOutcome, ExecuteError>) -> ExecutionOutcome {
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::error!(%error, "command execution failed");
            Err(error.to_string())
        }
    }
}
