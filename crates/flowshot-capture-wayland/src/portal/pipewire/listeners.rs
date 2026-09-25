//! The `PipeWire` stream listener callbacks and the consumer-side format
//! negotiation for [`super::capture_first_frames`].

use std::io::Cursor;

use pipewire::spa::buffer::DataType;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::VideoFormat;
use pipewire::spa::param::{ParamType, format_utils};
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{self, Pod};
use pipewire::spa::utils::{Fraction, Rectangle, SpaTypes};
use pipewire::stream::StreamState;

use super::super::error::PortalErrorKind;
use super::super::streams::frame_format_of;
use super::{RawPwFrame, Shared, UserData, lock_shared};

pub(super) fn on_state_changed(
    _stream: &pipewire::stream::Stream,
    user_data: &mut UserData,
    _old: StreamState,
    new: StreamState,
) {
    let StreamState::Error(message) = new else {
        return;
    };
    tracing::error!(
        node_id = user_data.node_id,
        %message,
        "PipeWire stream entered the error state"
    );
    {
        let mut shared = lock_shared(&user_data.shared);
        if shared.failure.is_none() {
            shared.failure = Some(PortalErrorKind::StreamFailed {
                node_id: user_data.node_id,
            });
        }
    }
    user_data.main_loop.quit();
}

pub(super) fn on_param_changed(
    _stream: &pipewire::stream::Stream,
    user_data: &mut UserData,
    id: u32,
    param: Option<&Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
        tracing::warn!(
            node_id = user_data.node_id,
            "unparsable stream format param"
        );
        return;
    };
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        return;
    }
    if let Err(error) = user_data.format.parse(param) {
        tracing::warn!(
            node_id = user_data.node_id,
            ?error,
            "failed to parse the negotiated raw video format"
        );
    }
}

pub(super) fn on_process(stream: &pipewire::stream::Stream, user_data: &mut UserData) {
    let mut shared = lock_shared(&user_data.shared);
    if shared.failure.is_some()
        || shared.timed_out
        || shared
            .frames
            .iter()
            .any(|frame| frame.node_id == user_data.node_id)
    {
        // One-shot discipline: the first frame per stream is the capture.
        return;
    }
    match read_first_frame(stream, user_data, &mut shared) {
        FrameRead::Captured => {
            shared.pending = shared.pending.saturating_sub(1);
            if shared.pending == 0 {
                drop(shared);
                user_data.main_loop.quit();
            }
        }
        FrameRead::WaitForNextBuffer => {}
        FrameRead::Failed(failure) => {
            shared.failure = Some(failure);
            drop(shared);
            user_data.main_loop.quit();
        }
    }
}

/// Why a `process` callback did not capture a frame.
enum FrameRead {
    /// The frame was copied into the shared state.
    Captured,
    /// The buffer was unusable (empty, unmapped, format not yet negotiated);
    /// a later buffer may succeed, so keep waiting under the deadline.
    WaitForNextBuffer,
    /// A terminal failure; the loop quits.
    Failed(PortalErrorKind),
}

fn read_first_frame(
    stream: &pipewire::stream::Stream,
    user_data: &UserData,
    shared: &mut Shared,
) -> FrameRead {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return FrameRead::WaitForNextBuffer;
    };
    let datas = buffer.datas_mut();
    let Some(data) = datas.first_mut() else {
        return FrameRead::WaitForNextBuffer;
    };
    let data_type = data.type_();
    if data_type == DataType::DmaBuf {
        return FrameRead::Failed(PortalErrorKind::DmabufUnsupported);
    }
    if data_type != DataType::MemFd && data_type != DataType::MemPtr && data_type != DataType::MemId
    {
        return FrameRead::WaitForNextBuffer;
    }
    let chunk = data.chunk();
    let (offset, size, stride) = (chunk.offset(), chunk.size(), chunk.stride());
    if size == 0 {
        return FrameRead::WaitForNextBuffer;
    }
    let Some(mapped) = data.data() else {
        return FrameRead::Failed(PortalErrorKind::IncompleteFrame);
    };
    let (Ok(start), Ok(len)) = (usize::try_from(offset), usize::try_from(size)) else {
        return FrameRead::Failed(PortalErrorKind::Internal(
            "buffer chunk geometry overflows usize",
        ));
    };
    let Some(end) = start.checked_add(len) else {
        return FrameRead::Failed(PortalErrorKind::IncompleteFrame);
    };
    let Some(bytes) = mapped.get(start..end) else {
        return FrameRead::Failed(PortalErrorKind::IncompleteFrame);
    };
    let rect = user_data.format.size();
    if rect.width == 0 || rect.height == 0 {
        // Format not negotiated yet (param_changed precedes process in
        // practice; be safe and wait for the next buffer).
        return FrameRead::WaitForNextBuffer;
    }
    let Some(format) = frame_format_of(user_data.format.format()) else {
        return FrameRead::Failed(PortalErrorKind::UnsupportedFormat {
            format: format!("{:?}", user_data.format.format()),
        });
    };
    let stride = u32::try_from(stride)
        .ok()
        .filter(|stride| *stride > 0)
        .unwrap_or_else(|| rect.width.saturating_mul(4));
    shared.frames.push(RawPwFrame {
        node_id: user_data.node_id,
        width: rect.width,
        height: rect.height,
        stride,
        format,
        data: bytes.to_vec(),
    });
    tracing::debug!(
        node_id = user_data.node_id,
        width = rect.width,
        height = rect.height,
        stride,
        ?format,
        "captured the first portal screencast frame"
    );
    FrameRead::Captured
}

/// Builds the consumer-side `EnumFormat` parameter: raw 32-bpp RGB formats,
/// no modifier property (requesting modifiers is what would make producers
/// allocate `dma-buf`), a size range covering any v1 display, and a
/// framerate range (irrelevant for a single frame, required by the
/// negotiation).
pub(super) fn enum_format_param() -> Result<Vec<u8>, PortalErrorKind> {
    let object = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA,
        ),
        pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 60, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    let (writer, _size) =
        PodSerializer::serialize(Cursor::new(Vec::new()), &pod::Value::Object(object))
            .map_err(|_| PortalErrorKind::Internal("format pod serialization failed"))?;
    Ok(writer.into_inner())
}
