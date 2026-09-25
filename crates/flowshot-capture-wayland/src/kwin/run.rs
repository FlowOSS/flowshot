//! The `ScreenShot2` capture orchestration: output enumeration, per-output
//! `CaptureArea` runs, single-image runs, and frame assembly.
//!
//! # Execution model
//!
//! Same one-shot worker discipline as the portal backends: the blocking
//! entry points run on a worker thread ([`spawn_worker`]), the async
//! `D-Bus` half on a thread-private `tokio` runtime, and the pipe readback
//! (blocking `poll`/`read`) directly on the worker thread between runtime
//! phases. `zbus` itself uses its `async-io` reactor (a background thread),
//! so it is drivable from the private runtime without feature coupling to
//! `tokio`. The wire mechanics (connection targets, method calls, reply
//! classification, payload readback) live in [`wire`].
//!
//! [`spawn_worker`]: crate::worker::spawn_worker
//! [`wire`]: crate::kwin::wire

use std::future::Future;
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

use flowshot_capture::{Frame, OutputRef};
use flowshot_core::geometry::{OutputInfo, Transform};
use wayland_client::Connection as WaylandConnection;
use zbus::{Connection, Message};

use super::error::KwinError;
use super::meta::{RawFrameMeta, frame_buffer, to_native_orientation};
use super::wire::{self, KwinBus, Request};
use crate::error::BackendError;
use crate::error::socket_connect_error;
use crate::icc::wait::collect_deadline_with;
use crate::session::CaptureState;
use crate::stitch::{CapturedOutputs, round_to_i32};

/// Per-phase deadline for machine-speed phases (connect, one capture call,
/// one payload readback): draft F27 parity with the other backends.
pub(crate) const KWIN_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for interactive captures: the human picker lives inside the
/// `D-Bus` call (the portal `HANDSHAKE_TIMEOUT` precedent).
pub(crate) const KWIN_INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(60);

/// Which outputs a capture run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selection {
    /// Every enumerated output, in registry-name order.
    All,
    /// The single output with this connector name.
    Named(String),
}

/// Runs one complete per-output capture: Wayland output enumeration, then
/// one `CaptureArea` call per selected output.
///
/// # Errors
///
/// Any [`KwinError`] of the chain: connect/collection failures, `D-Bus`
/// errors, decode failures, timeouts, and dimension mismatches.
pub(crate) fn run_capture(
    bus: KwinBus,
    selection: &Selection,
    paint_cursor: bool,
) -> Result<CapturedOutputs, KwinError> {
    let outputs = collect_outputs()?;
    let selected = select_outputs(outputs, selection)?;
    run_selected(bus, selected, paint_cursor)
}

/// Captures the given outputs sequentially over one `D-Bus` connection.
///
/// # Errors
///
/// Any [`KwinError`] of the chain.
pub(crate) fn run_selected(
    bus: KwinBus,
    outputs: Vec<OutputInfo>,
    paint_cursor: bool,
) -> Result<CapturedOutputs, KwinError> {
    let session = Session::connect(bus)?;
    let mut frames = Vec::with_capacity(outputs.len());
    for output in &outputs {
        frames.push(session.capture_output(output, paint_cursor)?);
    }
    Ok(CapturedOutputs { outputs, frames })
}

/// Runs one single-image capture (`CaptureActiveScreen`,
/// `CaptureActiveWindow`, or `CaptureInteractive`).
///
/// The returned frame is upright as `KWin` rendered it
/// ([`OutputRef::Composite`], [`Transform::Normal`]) with the metadata's
/// device-pixel scale - single-image captures are not anchored to the
/// enumerated layout.
///
/// # Errors
///
/// Any [`KwinError`] of the chain.
pub(crate) fn run_single(
    bus: KwinBus,
    request: Request,
    paint_cursor: bool,
) -> Result<Frame, KwinError> {
    let session = Session::connect(bus)?;
    let (meta, data) = session.pipe_capture(&request, paint_cursor)?;
    if let Some(screen) = &meta.screen {
        tracing::debug!(screen = %screen.0, "KWin reported the captured screen");
    }
    let buffer = frame_buffer(&meta, data)?;
    Ok(Frame {
        buffer,
        output: OutputRef::Composite,
        scale: meta.scale,
        transform: Transform::Normal,
    })
}

/// A live `ScreenShot2` session: the worker-private `tokio` runtime and the
/// `D-Bus` connection driven on it.
///
/// Teardown is deterministic by `Drop`: `zbus` (async-io reactor) runs its
/// socket reader task on the global `async-executor` pool, NOT on this
/// runtime, and its `Connection` has no drop-time close - merely dropping
/// the handle would leave the socket descriptor open with the reader task
/// parked on it. [`Drop`] therefore calls `Connection::close` (a
/// `shutdown(Both)` on the socket), which resolves the parked read
/// immediately, so the reader task ends and releases the descriptor without
/// depending on the peer. The runtime outlives the close call (fields drop
/// after `Drop::drop`).
struct Session {
    connection: Option<Connection>,
    runtime: tokio::runtime::Runtime,
}

impl Session {
    /// Connects to the target bus under the connect deadline.
    fn connect(bus: KwinBus) -> Result<Self, KwinError> {
        let runtime = crate::portal::run::build_runtime::<KwinError>()?;
        let connection = runtime.block_on(with_deadline(KWIN_TIMEOUT, wire::connect(bus)))?;
        Ok(Self {
            connection: Some(connection),
            runtime,
        })
    }

    /// One `ScreenShot2` method call under the request's budget. The caller
    /// owns (and must close) the pipe's write end.
    fn call(
        &self,
        request: &Request,
        paint_cursor: bool,
        write_end: BorrowedFd<'_>,
    ) -> Result<Message, KwinError> {
        let Some(connection) = self.connection.as_ref() else {
            return Err(KwinError::Internal(
                "the session connection is already closed",
            ));
        };
        self.runtime.block_on(with_deadline(
            budget_for(request),
            wire::call_capture(connection, request, paint_cursor, write_end),
        ))
    }

    /// One output's `CaptureArea` round-trip: pipe, call, metadata guard,
    /// payload readback, inverse remap into the shared [`Frame`] contract.
    fn capture_output(&self, output: &OutputInfo, paint_cursor: bool) -> Result<Frame, KwinError> {
        let request = area_request(output)?;
        let (read_end, write_end) = wire::open_pipe()?;
        let replied = self.call(&request, paint_cursor, write_end.as_fd());
        drop(write_end);
        let reply = replied?;
        let meta = wire::reply_metadata(&reply)?;
        guard_dimensions(&meta, output)?;
        let data = wire::read_payload(read_end.as_fd(), meta.expected_bytes, KWIN_TIMEOUT)?;
        let buffer = frame_buffer(&meta, data)?;
        tracing::debug!(
            output = %output.connector,
            width = meta.width,
            height = meta.height,
            qimage_format = meta.qimage_format,
            "KWin ScreenShot2 delivered an area capture"
        );
        let buffer = to_native_orientation(buffer, output.transform)?;
        Ok(Frame {
            buffer,
            output: OutputRef::from(output),
            scale: output.scale,
            transform: output.transform,
        })
    }

    /// The single-image pipe round-trip shared by [`run_single`].
    fn pipe_capture(
        &self,
        request: &Request,
        paint_cursor: bool,
    ) -> Result<(RawFrameMeta, Vec<u8>), KwinError> {
        let (read_end, write_end) = wire::open_pipe()?;
        let replied = self.call(request, paint_cursor, write_end.as_fd());
        drop(write_end);
        let reply = replied?;
        let meta = wire::reply_metadata(&reply)?;
        tracing::debug!(
            width = meta.width,
            height = meta.height,
            qimage_format = meta.qimage_format,
            "KWin ScreenShot2 delivered a single-image capture"
        );
        let data = wire::read_payload(read_end.as_fd(), meta.expected_bytes, budget_for(request))?;
        Ok((meta, data))
    }
}

/// The budget for one request: interactive captures wait on a human picker
/// inside the `D-Bus` call (the portal `HANDSHAKE_TIMEOUT` precedent);
/// everything else is machine speed.
fn budget_for(request: &Request) -> Duration {
    match request {
        Request::Interactive(_) => KWIN_INTERACTIVE_TIMEOUT,
        Request::ActiveScreen | Request::ActiveWindow | Request::Area { .. } => KWIN_TIMEOUT,
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        // Runs on the worker thread OUTSIDE any block_on (the Session lives
        // in the run functions' scope, never inside a runtime phase), so
        // driving the close here cannot nest runtimes. The close is bounded:
        // an uncontended async mutex plus a shutdown syscall.
        if let Err(error) = self.runtime.block_on(connection.close()) {
            tracing::debug!(%error, "the ScreenShot2 connection close reported an error");
        }
    }
}

/// Derives the `CaptureArea` request from an output's logical rectangle.
fn area_request(output: &OutputInfo) -> Result<Request, KwinError> {
    let rect = &output.logical_rect;
    let dimension = |value: f64| -> Result<u32, KwinError> {
        u32::try_from(round_to_i32(value))
            .map_err(|_| KwinError::Internal("output logical dimension is negative"))
    };
    Ok(Request::Area {
        x: round_to_i32(rect.x.0),
        y: round_to_i32(rect.y.0),
        width: dimension(rect.width.0)?,
        height: dimension(rect.height.0)?,
    })
}

/// Guards the upright capture dimensions against the output's post-transform
/// physical size (the pixel-space anchor the per-output frames promise).
fn guard_dimensions(meta: &RawFrameMeta, output: &OutputInfo) -> Result<(), KwinError> {
    let expected = output.buffer_size();
    let (Ok(width), Ok(height)) = (
        u32::try_from(expected.width.0),
        u32::try_from(expected.height.0),
    ) else {
        return Err(KwinError::Internal("output physical size is negative"));
    };
    if (meta.width, meta.height) == (width, height) {
        return Ok(());
    }
    Err(KwinError::BufferSizeMismatch {
        reported: (meta.width, meta.height),
        expected: (expected.width.0, expected.height.0),
    })
}

/// Enumerates the session's outputs on a one-shot Wayland connection (the
/// portal backends' anchor discipline: `ScreenShot2` delivers images without
/// layout metadata this crate trusts).
///
/// # Errors
///
/// The connect, timeout, transport, or protocol error of the collection.
pub(crate) fn collect_outputs() -> Result<Vec<OutputInfo>, KwinError> {
    let connection = WaylandConnection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = connection.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline_with::<KwinError>(
        &connection,
        &mut queue,
        &mut state,
        Instant::now() + KWIN_TIMEOUT,
    )?;
    Ok(state.snapshot().outputs)
}

/// Resolves the selection against the enumerated outputs.
///
/// # Errors
///
/// [`KwinError::NoOutputs`] when the session has no outputs and
/// [`KwinError::OutputNotFound`] for a connector name the session does not
/// advertise (listing the available connectors).
pub(crate) fn select_outputs(
    outputs: Vec<OutputInfo>,
    selection: &Selection,
) -> Result<Vec<OutputInfo>, KwinError> {
    if outputs.is_empty() {
        return Err(KwinError::NoOutputs);
    }
    match selection {
        Selection::All => Ok(outputs),
        Selection::Named(name) => {
            let available = outputs
                .iter()
                .map(|output| output.connector.clone())
                .collect();
            let found = outputs
                .into_iter()
                .find(|output| &output.connector == name)
                .ok_or_else(|| KwinError::OutputNotFound {
                    requested: name.clone(),
                    available,
                })?;
            Ok(vec![found])
        }
    }
}

/// Runs `work` under a hard budget, mapping an expired deadline onto the
/// backend's timeout error and dropping (cancelling) the work future. Must
/// run inside the worker's `tokio` runtime.
async fn with_deadline<E, T>(
    budget: Duration,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E>
where
    E: BackendError,
{
    tokio::select! {
        result = work => result,
        () = tokio::time::sleep(budget) => {
            tracing::warn!(?budget, "KWin ScreenShot2 phase exceeded its deadline; cancelling");
            Err(E::timeout(budget))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales and sizes are exact integral literals (geometry-notepad
    // convention), so strict comparison is the intended assertion.
    #![allow(clippy::float_cmp)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize};

    use super::*;
    use crate::kwin::meta::qimage_layout;

    #[test]
    fn area_request_rounds_the_logical_rect() {
        let output = OutputInfo::new(
            "DP-1",
            "DP-1",
            LogicalRect::new(
                Logical(10.4),
                Logical(-20.6),
                Logical(1920.5),
                Logical(1080.0),
            ),
            PhysicalSize::new(PhysicalPx(1920), PhysicalPx(1080)),
            1.0,
            Transform::Normal,
        )
        .unwrap();
        match area_request(&output).unwrap() {
            Request::Area {
                x,
                y,
                width,
                height,
            } => {
                assert_eq!((x, y), (10, -21));
                assert_eq!((width, height), (1921, 1080));
            }
            other => panic!("expected Area, got {other:?}"),
        }
    }

    #[test]
    fn dimension_guard_accepts_the_post_transform_physical_size() {
        let output = OutputInfo::new(
            "R",
            "R",
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(4.0), Logical(3.0)),
            PhysicalSize::new(PhysicalPx(3), PhysicalPx(4)),
            1.0,
            Transform::Rot90,
        )
        .unwrap();
        let matching = RawFrameMeta {
            layout: qimage_layout(5).unwrap(),
            qimage_format: 5,
            width: 4,
            height: 3,
            stride: 16,
            scale: 1.0,
            expected_bytes: 48,
            screen: None,
        };
        assert!(guard_dimensions(&matching, &output).is_ok());
        let mismatch = RawFrameMeta {
            width: 3,
            height: 4,
            ..matching
        };
        assert!(matches!(
            guard_dimensions(&mismatch, &output),
            Err(KwinError::BufferSizeMismatch { .. })
        ));
    }

    #[test]
    fn selection_rejects_unknown_connectors_listing_available() {
        let output = OutputInfo::new(
            "DP-1",
            "DP-1",
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(4.0), Logical(3.0)),
            PhysicalSize::new(PhysicalPx(4), PhysicalPx(3)),
            1.0,
            Transform::Normal,
        )
        .unwrap();
        let err = select_outputs(vec![output], &Selection::Named("DP-99".to_owned())).unwrap_err();
        match err {
            KwinError::OutputNotFound {
                requested,
                available,
            } => {
                assert_eq!(requested, "DP-99");
                assert_eq!(available, vec!["DP-1".to_owned()]);
            }
            other => panic!("expected OutputNotFound, got {other:?}"),
        }
        assert!(matches!(
            select_outputs(vec![], &Selection::All),
            Err(KwinError::NoOutputs)
        ));
    }

    #[test]
    fn deadline_expiry_surfaces_the_backend_timeout() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let budget = Duration::from_millis(20);
        let result = runtime.block_on(with_deadline::<KwinError, ()>(
            budget,
            std::future::pending(),
        ));
        assert!(matches!(
            result.unwrap_err(),
            KwinError::Timeout { timeout } if timeout == budget
        ));
    }
}
