//! The `ext-image-copy-capture-v1` pointer-cursor session: cursor position,
//! cursor image, and the event stream bridged onto
//! [`CursorStream`](flowshot_capture::CursorStream).
//!
//! A pointer-cursor session is created per output through
//! `ext_image_copy_capture_manager_v1.create_pointer_cursor_session(source,
//! pointer)` and reports `enter`/`leave`/`position`/`hotspot` for the cursor
//! over that output, plus an embedded capture session for the cursor image
//! (`get_capture_session`). [`IccBackend::cursor_pos`] is the one-shot global
//! logical position query; [`IccBackend::cursor_image`] captures the cursor
//! pixels; [`IccBackend::cursor_events`](flowshot_capture::CaptureBackend::cursor_events)
//! returns the long-lived event stream.
//!
//! # Coordinate space
//!
//! The protocol reports `position` in source-local post-transform physical
//! (buffer) coordinates; [`source_local_to_global`] converts it into the global
//! logical layout space using the reporting output's scale and origin.
//!
//! # Degradation
//!
//! Cursor observation is never a capture requirement. A `Hyprland`
//! `PERMISSION_TYPE_CURSOR_POS` denial leaves the session inert (no position
//! event), and every failure path collapses to `None` with a warning - see
//! the private `run` module.

pub(crate) mod dispatch;
pub mod protocol;
pub(crate) mod run;
pub(crate) mod stream;

use flowshot_core::geometry::{LogicalPoint, ToPhysical};

use crate::icc::IccBackend;

pub use protocol::{CursorImage, composite_cursor_rgba, source_local_to_global};

impl IccBackend {
    /// Queries the cursor position once, in global logical layout space.
    ///
    /// Creates a pointer-cursor session per output, waits up to 500 ms for the
    /// first `position` event, converts it with the reporting output's
    /// geometry, and tears the sessions down. Returns `None` - never an error -
    /// when the cursor overlaps no output, the seat has no pointer, a capture
    /// manager is absent, or the compositor denied cursor-position permission.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use flowshot_capture_wayland::IccBackend;
    /// # futures::executor::block_on(async {
    /// let backend = IccBackend::new();
    /// if let Some((x, y)) = backend.cursor_pos().await {
    ///     println!("cursor at logical ({x}, {y})");
    /// }
    /// # });
    /// ```
    pub async fn cursor_pos(&self) -> Option<(i32, i32)> {
        run_on_worker("flowshot-cursor-pos", run::cursor_pos)
            .await
            .map(round_pair)
    }

    /// Captures the cursor image once through the embedded capture session of
    /// the output the cursor currently overlaps.
    ///
    /// Returns the `RGBA8888` pixels, dimensions, and hotspot, or `None` when
    /// the cursor is unavailable (no position, denied permission, or a frame
    /// failure). This feeds the client-side compositing fallback
    /// ([`composite_cursor_rgba`]); the `ext-image-copy-capture-v1` backend
    /// itself paints cursors via the session option and does not need it.
    pub async fn cursor_image(&self) -> Option<CursorImage> {
        run_on_worker("flowshot-cursor-image", run::cursor_image).await
    }
}

/// Rounds a global logical point to the nearest integer pixel pair (half away
/// from zero, saturating), matching `hyprctl cursorpos`'s integer report.
/// Shared with the Hyprland IPC layer so both position sources round alike.
pub(crate) fn round_pair(logical: LogicalPoint) -> (i32, i32) {
    (logical.x.to_physical(1.0).0, logical.y.to_physical(1.0).0)
}

/// Runs one blocking cursor query on a dedicated worker thread and bridges the
/// result into a non-blocking future, mirroring the frame-capture worker. A
/// dropped future leaves the worker to finish and tear its one-shot connection
/// down; a panicked worker closes the channel and yields `None`. Shared with
/// the layered resolver's Hyprland IPC query.
pub(crate) async fn run_on_worker<T, F>(name: &str, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> Option<T> + Send + 'static,
{
    let (sender, receiver) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            if sender.send(work()).is_err() {
                tracing::debug!("cursor query result discarded: the future was cancelled");
            }
        });
    match spawned {
        Ok(_worker) => receiver.await.ok().flatten(),
        Err(error) => {
            tracing::warn!(%error, "could not spawn the cursor query worker");
            None
        }
    }
}
