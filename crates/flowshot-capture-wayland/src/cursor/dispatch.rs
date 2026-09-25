//! The [`Dispatch`](wayland_client::Dispatch) implementation for
//! `ext_image_copy_capture_cursor_session_v1`.
//!
//! Every event fills the plain-data [`ActiveCursor`] sink (so one-shot queries
//! can read the first reported position) and, when a [`CursorSink`] is
//! installed, is bridged onto the [`CursorStream`] as it arrives - event-driven,
//! never polled. Cursor-session proxies carry their output index as user data
//! so a `position` is converted with the geometry of the output that reported
//! it.
//!
//! [`CursorStream`]: flowshot_capture::CursorStream

use flowshot_capture::CursorEvent;
use flowshot_core::geometry::PhysicalPoint;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_cursor_session_v1::{
    self, ExtImageCopyCaptureCursorSessionV1,
};

use super::protocol::source_local_to_global;
use crate::session::CaptureState;

/// User data of a cursor-session proxy: the index of the output it was
/// created from, into [`CaptureState::cursor_layout`].
pub(crate) type CursorSessionId = usize;

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, CursorSessionId> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureCursorSessionV1,
        event: <ExtImageCopyCaptureCursorSessionV1 as wayland_client::Proxy>::Event,
        data: &CursorSessionId,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let index = *data;
        match event {
            ext_image_copy_capture_cursor_session_v1::Event::Enter => {
                if let Some(session) = state.cursor.session_mut(index) {
                    session.entered = true;
                }
                forward(state, CursorEvent::Entered);
            }
            ext_image_copy_capture_cursor_session_v1::Event::Leave => {
                forward(state, CursorEvent::Left);
            }
            ext_image_copy_capture_cursor_session_v1::Event::Position { x, y } => {
                let local = PhysicalPoint::from_raw(x, y);
                if let Some(session) = state.cursor.session_mut(index) {
                    session.position = Some(local);
                }
                if let Some(output) = state.cursor_layout.get(index) {
                    forward(
                        state,
                        CursorEvent::Moved {
                            position: source_local_to_global(local, output),
                        },
                    );
                }
            }
            ext_image_copy_capture_cursor_session_v1::Event::Hotspot { x, y } => {
                let offset = PhysicalPoint::from_raw(x, y);
                if let Some(session) = state.cursor.session_mut(index) {
                    session.hotspot = Some(offset);
                }
                forward(state, CursorEvent::Hotspot { offset });
            }
            // The generated event enum is #[non_exhaustive]; the cursor
            // session has no other events in v1.
            _ => {}
        }
    }
}

/// Sends one event to the installed cursor sink, ignoring a closed channel
/// (the consumer dropped the stream; the worker's shutdown path handles it).
fn forward(state: &mut CaptureState, event: CursorEvent) {
    if let Some(sink) = &state.cursor_sink
        && sink.unbounded_send(event).is_err()
    {
        tracing::debug!("cursor stream consumer is gone; events are dropped");
    }
}
