//! [`Dispatch`] implementations for the `wlr-screencopy-unstable-v1` object
//! chain.
//!
//! Every event only fills the plain-data [`ActiveScreencopy`] sink in
//! [`CaptureState`]; requests are issued exclusively by the capture runner
//! between dispatch phases. The frame proxy carries a [`ScreencopyGen`]
//! generation tag so events from a destroyed predecessor frame (still buffered
//! in the socket when the next per-output capture starts) are dropped instead
//! of corrupting the new capture's state.
//!
//! [`ActiveScreencopy`]: super::protocol::ActiveScreencopy

use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, Flags, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::screencopy::protocol::{AdvertisedBuffer, FramePhase};
use crate::session::CaptureState;

/// Generation tag: user data of the active capture's frame proxy. Events whose
/// tag differs from [`ActiveScreencopy::generation`] belong to a destroyed
/// predecessor and are dropped.
///
/// [`ActiveScreencopy::generation`]: super::protocol::ActiveScreencopy::generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreencopyGen(pub u32);

impl CaptureState {
    /// Records one frame event when the generation tag is current.
    fn screencopy_frame_event(
        &mut self,
        generation: ScreencopyGen,
        event: &zwlr_screencopy_frame_v1::Event,
    ) {
        if generation.0 != self.screencopy.generation {
            return;
        }
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let (shm_format, wire_format) = match format {
                    WEnum::Value(known) => (Some(*known), u32::from(*known)),
                    WEnum::Unknown(value) => (None, *value),
                };
                self.screencopy.buffer = Some(AdvertisedBuffer {
                    shm_format,
                    wire_format,
                    width: *width,
                    height: *height,
                    stride: *stride,
                });
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                // y_invert is bit 0 of the flags bitfield; extract it from both
                // the known-variant and raw-unknown representations.
                self.screencopy.y_invert = match flags {
                    WEnum::Value(known) => known.contains(Flags::YInvert),
                    WEnum::Unknown(bits) => bits & 0x1 != 0,
                };
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => self.screencopy.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                self.screencopy.phase = FramePhase::Ready;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                self.screencopy.phase = FramePhase::Failed;
            }
            // Damage rectangles only accompany copy_with_damage (unused: a
            // one-shot full-frame copy damages nothing incrementally), and
            // linux-dmabuf advertisements are ignored by design (v1 captures
            // into wl_shm buffers only). The generated event enum is
            // #[non_exhaustive], so the wildcard also covers future events.
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrScreencopyManagerV1,
        _event: <ZwlrScreencopyManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyGen> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as wayland_client::Proxy>::Event,
        data: &ScreencopyGen,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.screencopy_frame_event(*data, &event);
    }
}
