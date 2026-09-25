//! The long-lived cursor-event stream: a dedicated worker thread owning its own
//! Wayland connection and a `calloop` event loop (the same shape as
//! [`CaptureThread`]), creating one pointer-cursor session per output and
//! bridging enter/leave/position/hotspot onto a [`CursorStream`].
//!
//! The worker is purely event-driven: `calloop`'s `WaylandSource` blocks on the
//! compositor socket and dispatches cursor-session events into
//! [`CaptureState::cursor_sink`], which forwards each as a [`CursorEvent`]. No
//! position is ever polled. Dropping the returned stream closes the shutdown
//! channel, which stops the loop and tears the connection (and every session)
//! down - so an abandoned stream cannot leak a thread or compositor sessions.
//!
//! [`CaptureThread`]: crate::CaptureThread

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use calloop::EventLoop;
use calloop::channel::{self, Channel, Sender};
use calloop_wayland_source::WaylandSource;
use flowshot_capture::{CursorEvent, CursorStream};
use futures::Stream;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use wayland_client::Connection;

use super::run::{CursorSessionHandles, create_cursor_sessions};
use crate::error::socket_connect_error;
use crate::icc::wait::{CAPTURE_TIMEOUT, collect_deadline};
use crate::session::CaptureState;

/// Thread name visible in debuggers and process listings.
const THREAD_NAME: &str = "flowshot-cursor";

/// Starts the cursor-event worker and returns its stream, or `None` when the
/// worker thread cannot be spawned.
///
/// Connection or session-setup failures inside the worker are not surfaced
/// here: the worker drops its event sender, so the returned stream simply ends
/// (a graceful degradation, never a capture failure).
pub(crate) fn spawn_cursor_stream() -> Option<CursorStream> {
    let (event_tx, event_rx) = unbounded::<CursorEvent>();
    let (shutdown_tx, shutdown_rx) = channel::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name(THREAD_NAME.to_owned())
        .spawn(move || run_worker(event_tx, shutdown_rx));
    match spawned {
        Ok(_worker) => Some(Box::pin(CursorStreamBridge {
            events: event_rx,
            _shutdown: shutdown_tx,
        })),
        Err(error) => {
            tracing::warn!(%error, "could not spawn the cursor-event worker");
            None
        }
    }
}

/// The stream handle: yields bridged cursor events and, on drop, closes the
/// shutdown channel that stops the worker.
struct CursorStreamBridge {
    events: UnboundedReceiver<CursorEvent>,
    _shutdown: Sender<()>,
}

impl Stream for CursorStreamBridge {
    type Item = CursorEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<CursorEvent>> {
        Pin::new(&mut self.events).poll_next(cx)
    }
}

impl std::fmt::Debug for CursorStreamBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorStreamBridge").finish_non_exhaustive()
    }
}

/// Connects, creates the per-output cursor sessions, and runs the event loop
/// until the shutdown channel closes.
fn run_worker(event_tx: UnboundedSender<CursorEvent>, shutdown_rx: Channel<()>) {
    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(error) => {
            let hint = socket_connect_error(error);
            tracing::warn!(%hint, "cursor stream could not connect; the stream will end");
            return;
        }
    };
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    state.cursor_sink = Some(event_tx);
    if let Err(error) = collect_deadline(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    ) {
        tracing::warn!(%error, "cursor stream session collection failed; the stream will end");
        return;
    }
    // The handles must outlive the loop: dropping them destroys the sessions.
    let _handles: Vec<CursorSessionHandles> = match create_cursor_sessions(&mut queue, &mut state) {
        Ok(handles) => handles,
        Err(error) => {
            tracing::warn!(%error, "cursor stream could not create sessions; the stream will end");
            return;
        }
    };
    if let Err(error) = queue.flush() {
        tracing::warn!(%error, "cursor stream could not flush session creation");
        return;
    }

    let mut event_loop = match EventLoop::<CaptureState>::try_new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            tracing::warn!(%error, "cursor stream event loop failed to start");
            return;
        }
    };
    let handle = event_loop.handle();
    if let Err(insert) = WaylandSource::new(conn, queue).insert(handle.clone()) {
        tracing::warn!(error = %insert.error, "cursor stream could not register the Wayland source");
        return;
    }
    let stop = event_loop.get_signal();
    if let Err(insert) = handle.insert_source(shutdown_rx, move |event, (), _state| {
        if matches!(event, channel::Event::Msg(()) | channel::Event::Closed) {
            stop.stop();
            stop.wakeup();
        }
    }) {
        tracing::warn!(error = %insert.error, "cursor stream could not register the shutdown source");
        return;
    }
    if let Err(error) = event_loop.run(Option::<std::time::Duration>::None, &mut state, |_| {}) {
        tracing::debug!(%error, "the cursor stream event loop terminated");
    }
}
