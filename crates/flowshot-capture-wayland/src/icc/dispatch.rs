//! [`Dispatch`] implementations for the `ext-image-copy-capture-v1` object
//! chain and its `wl_shm` buffer plumbing.
//!
//! Every event only fills the plain-data [`ActiveCapture`] sink in
//! [`CaptureState`]; requests are issued exclusively by the capture runner
//! between dispatch phases. Session and frame proxies carry an [`IccGen`]
//! generation tag so events from a destroyed predecessor session (still
//! buffered in the socket when the next per-output capture starts) are
//! dropped instead of corrupting the new capture's state.

use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};

use crate::session::CaptureState;

/// Generation tag: user data of the active capture's session and frame
/// proxies. Events whose tag differs from [`ActiveCapture::generation`]
/// belong to a destroyed predecessor and are dropped.
///
/// [`ActiveCapture::generation`]: super::protocol::ActiveCapture::generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IccGen(pub u32);

impl CaptureState {
    /// Records one session event when the generation tag is current.
    fn session_event(
        &mut self,
        generation: IccGen,
        event: &ext_image_copy_capture_session_v1::Event,
    ) {
        if generation.0 != self.active.generation {
            return;
        }
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                self.active.buffer_size = Some((*width, *height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                let wire = match format {
                    WEnum::Value(known) => u32::from(*known),
                    WEnum::Unknown(value) => *value,
                };
                self.active.shm_formats.push(wire);
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                self.active.constraints_done = true;
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                tracing::debug!("ext-image-copy-capture session stopped by the compositor");
                self.active.stopped = true;
            }
            // v1 captures into wl_shm buffers only: dma-buf advertisements
            // are ignored by design (todo 7 scope: SHM only). The generated
            // event enum is #[non_exhaustive].
            _ => {}
        }
    }

    /// Records one frame event when the generation tag is current.
    fn frame_event(&mut self, generation: IccGen, event: &ext_image_copy_capture_frame_v1::Event) {
        if generation.0 != self.active.generation {
            return;
        }
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform { transform } => {
                self.active.frame_transform = match transform {
                    WEnum::Value(known) => Some(u32::from(*known)),
                    WEnum::Unknown(value) => {
                        tracing::warn!(value, "unknown frame transform wire value");
                        None
                    }
                };
            }
            ext_image_copy_capture_frame_v1::Event::Ready => self.active.ready = true,
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                self.active.failed = Some(match reason {
                    WEnum::Value(known) => u32::from(*known),
                    WEnum::Unknown(value) => *value,
                });
            }
            // Damage rectangles are informational for a one-shot full-frame
            // capture (the client damaged the whole buffer); presentation
            // timestamps are not part of the v1 Frame contract. The
            // generated event enum is #[non_exhaustive].
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.roundtrip_pending = false;
        }
    }
}

impl Dispatch<WlShm, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // wl_shm.format advertisements are superseded by the per-session
        // shm_format constraint events; nothing to record.
    }
}

impl Dispatch<WlShmPool, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShmPool,
        _event: <WlShmPool as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // wl_shm_pool has no events.
    }
}

impl Dispatch<WlBuffer, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: <WlBuffer as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The protocol states wl_buffer.release is unused for capture
        // frames; the one-shot buffer dies with the capture anyway.
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtImageCopyCaptureManagerV1,
        _event: <ExtImageCopyCaptureManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtOutputImageCaptureSourceManagerV1,
        _event: <ExtOutputImageCaptureSourceManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The source manager has no events.
    }
}

impl Dispatch<ExtImageCaptureSourceV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtImageCaptureSourceV1,
        _event: <ExtImageCaptureSourceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The capture source has no events.
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, IccGen> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureSessionV1,
        event: <ExtImageCopyCaptureSessionV1 as wayland_client::Proxy>::Event,
        data: &IccGen,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.session_event(*data, &event);
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, IccGen> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureFrameV1,
        event: <ExtImageCopyCaptureFrameV1 as wayland_client::Proxy>::Event,
        data: &IccGen,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.frame_event(*data, &event);
    }
}
