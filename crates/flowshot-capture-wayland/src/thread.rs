//! The dedicated capture thread owning the Wayland connection and its
//! `calloop` event loop.
//!
//! All wayland dispatching happens on this thread; owning threads only
//! exchange bounded request/reply messages with it, so a slow or frozen
//! compositor can never block a foreign event loop.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use calloop::EventLoop;
use calloop::channel::{self, Channel, SyncSender};
use calloop_wayland_source::WaylandSource;
use flowshot_capture::CapabilityProbe;
use flowshot_core::geometry::OutputInfo;
use wayland_client::Connection;

use crate::error::{ConnectError, ProbeError, socket_connect_error};
use crate::session::{CaptureState, SessionSnapshot, collect_session};

/// Thread name visible in debuggers and process listings.
const THREAD_NAME: &str = "flowshot-capture";
/// Bound on pending commands before `send` applies backpressure to callers.
const COMMAND_QUEUE: usize = 8;
/// Deadline for the capture thread's initial session setup.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Deadline for in-session requests.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Commands delivered to the capture thread's event loop.
#[derive(Debug)]
enum Command {
    /// Answer with a fresh snapshot of the session state.
    Snapshot {
        /// Where to deliver the snapshot.
        reply: mpsc::Sender<SessionSnapshot>,
    },
    /// Stop the event loop and end the thread.
    Shutdown,
}

/// A Wayland capture session running on its own thread.
///
/// [`CaptureThread::spawn`] connects to the compositor (a dedicated
/// connection, separate from any UI toolkit's - see the crate docs),
/// collects the registry globals and output geometry, and then parks the
/// thread on a `calloop` event loop that keeps the session state current
/// (output hotplug included). Request methods answer from that live state;
/// they block the caller only up to a bounded timeout.
///
/// Dropping the thread handle shuts the capture thread down.
#[derive(Debug)]
pub struct CaptureThread {
    commands: SyncSender<Command>,
    join: Option<JoinHandle<()>>,
}

impl CaptureThread {
    /// Connects to the Wayland compositor from the environment and starts
    /// the capture thread.
    ///
    /// Blocks until the thread finished its startup session probe (registry
    /// listing plus output geometry), bounded by a 10 second deadline.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectError::Socket`] with an environment-based hint when
    /// no compositor socket is reachable, [`ConnectError::Setup`] when the
    /// session probe fails, [`ConnectError::StartupTimeout`] when the thread
    /// does not report in time, and [`ConnectError::ThreadSpawn`] when the
    /// operating system refuses the thread.
    pub fn spawn() -> Result<Self, ConnectError> {
        let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
        let (commands, command_source) = channel::sync_channel::<Command>(COMMAND_QUEUE);
        let (startup, startup_result) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || run_thread(conn, command_source, &startup))?;
        match startup_result.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                commands,
                join: Some(join),
            }),
            Ok(Err(err)) => {
                let _ = join.join();
                Err(ConnectError::Setup(err))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ConnectError::StartupTimeout {
                timeout: STARTUP_TIMEOUT,
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(ConnectError::Setup(ProbeError::ThreadClosed))
            }
        }
    }

    /// Requests a fresh snapshot of the session: protocol globals, the
    /// capability probe, and the enumerated outputs.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError::Timeout`] when the capture thread does not
    /// answer within 10 seconds and [`ProbeError::ThreadClosed`] when the
    /// thread is gone.
    pub fn snapshot(&self) -> Result<SessionSnapshot, ProbeError> {
        let (reply, replies) = mpsc::channel();
        self.commands
            .send(Command::Snapshot { reply })
            .map_err(|_| ProbeError::ThreadClosed)?;
        match replies.recv_timeout(REPLY_TIMEOUT) {
            Ok(snapshot) => Ok(snapshot),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ProbeError::Timeout {
                timeout: REPLY_TIMEOUT,
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(ProbeError::ThreadClosed),
        }
    }

    /// The capability probe derived from the live session state.
    ///
    /// # Errors
    ///
    /// Same transport errors as [`CaptureThread::snapshot`].
    pub fn probe(&self) -> Result<CapabilityProbe, ProbeError> {
        Ok(self.snapshot()?.probe)
    }

    /// The outputs visible to this session, in registry-name order.
    ///
    /// # Errors
    ///
    /// Same transport errors as [`CaptureThread::snapshot`].
    pub fn outputs(&self) -> Result<Vec<OutputInfo>, ProbeError> {
        Ok(self.snapshot()?.outputs)
    }

    /// Stops the capture thread and waits for it to exit. Also runs on
    /// drop; calling it explicitly only makes the teardown point visible.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(join) = self.join.take()
            && let Err(panic) = join.join()
        {
            tracing::error!(?panic, "the Wayland capture thread panicked");
        }
    }
}

impl Drop for CaptureThread {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Builds the event loop, runs it, and reports setup failures through the
/// startup channel.
fn run_thread(
    conn: Connection,
    command_source: Channel<Command>,
    startup: &mpsc::Sender<Result<(), ProbeError>>,
) {
    let (snapshot, mut state, mut event_loop) = match build_loop(conn, command_source) {
        Ok(built) => built,
        Err(err) => {
            let _ = startup.send(Err(err));
            return;
        }
    };
    if startup.send(Ok(())).is_err() {
        // The spawner gave up (startup timeout); do not linger.
        return;
    }
    tracing::debug!(
        outputs = snapshot.outputs.len(),
        desktop = ?snapshot.probe.desktop,
        "Wayland capture session ready on its dedicated thread"
    );
    if let Err(err) = event_loop.run(Option::<Duration>::None, &mut state, |_| {}) {
        tracing::error!(%err, "the Wayland capture event loop terminated with an error");
    }
}

/// Connects the pieces: startup collection on the queue, then the queue and
/// the command channel as `calloop` sources.
fn build_loop(
    conn: Connection,
    command_source: Channel<Command>,
) -> Result<
    (
        SessionSnapshot,
        CaptureState,
        EventLoop<'static, CaptureState>,
    ),
    ProbeError,
> {
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_session(&conn, &mut queue, &mut state)?;
    let snapshot = state.snapshot();

    let event_loop = EventLoop::<CaptureState>::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn, queue)
        .insert(handle.clone())
        .map_err(|err| ProbeError::EventLoop(err.error))?;
    let stop = event_loop.get_signal();
    handle
        .insert_source(command_source, move |event, (), state| match event {
            channel::Event::Msg(Command::Snapshot { reply }) => {
                let _ = reply.send(state.snapshot());
            }
            channel::Event::Msg(Command::Shutdown) | channel::Event::Closed => {
                stop.stop();
                stop.wakeup();
            }
        })
        .map_err(|err| ProbeError::EventLoop(err.error))?;
    Ok((snapshot, state, event_loop))
}
