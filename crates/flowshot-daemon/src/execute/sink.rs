//! The daemon-side execution seams (todo 38): the [`ExecutingSink`] that
//! replaces the todo-32 `LoggingSink` default, the window-session
//! [`Heartbeat`] that keeps an auto-spawned daemon alive while a session
//! child is open, and the `flowshot pin` last-capture resolution.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{ExecCtx, ExecOutcome, ExecuteError, pin};
use crate::command::{CommandSink, DaemonCommand};

/// The heartbeat interval: comfortably inside the smallest sane idle
/// grace so an open window session never idles the daemon out.
const HEARTBEAT: Duration = Duration::from_secs(20);

/// Keeps the auto-spawned daemon's idle clock fresh while a window
/// session (overlay / launcher / settings) is open; dropping it stops the
/// beat. The four Amendment-#3 persistence reasons stay untouched - an
/// in-flight session is transient ACTIVITY (touch), not a residency
/// reason. No-op in the one-shot CLI process (no daemon state).
#[derive(Debug)]
pub struct Heartbeat {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Heartbeat {
    /// Starts the beat for a daemon-resident context.
    #[must_use]
    pub fn start(ctx: &ExecCtx) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let Some(state) = ctx.state.clone() else {
            return Self { stop, worker: None };
        };
        let flag = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("flowshot-heartbeat".to_owned())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    state.touch(Instant::now());
                    std::thread::sleep(HEARTBEAT);
                }
            })
            .ok();
        Self { stop, worker }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
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

impl ExecutingSink {
    /// A sink executing against `ctx`.
    #[must_use]
    pub const fn new(ctx: ExecCtx) -> Self {
        Self { ctx }
    }
}

impl CommandSink for ExecutingSink {
    fn dispatch(&self, command: DaemonCommand) {
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("flowshot-exec-{command}"))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        if let Err(error) = runtime.block_on(super::execute(command, &ctx)) {
                            tracing::error!(%error, "command execution failed");
                        }
                    }
                    Err(error) => tracing::error!(%error, "executor runtime build failed"),
                }
            });
        if let Err(error) = spawned {
            tracing::error!(%error, "executor thread spawn failed");
        }
    }
}
