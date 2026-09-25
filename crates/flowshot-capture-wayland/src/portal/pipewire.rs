//! Single-frame `PipeWire` capture for the `ScreenCast` portal backend.
//!
//! Connects a `PipeWire` client through the portal-provided remote fd,
//! creates one capture stream per portal stream node
//! (`media.type=Video`, `media.category=Capture`, `media.role=Screen`),
//! negotiates a raw 32-bpp format (no `dma-buf` modifiers, so producers
//! allocate `MemFd` shared memory - the crate-wide v1 SHM-only policy),
//! copies the FIRST frame per stream, and quits the loop. Teardown is
//! deterministic drop order (listeners -> streams -> core -> context ->
//! loop): destroying the core disconnects the client, which removes every
//! node this capture created from the graph (the QA leak assert).
//!
//! The deadline is a `spa` loop timer armed for the remaining budget: the
//! `PipeWire` main loop is blocking C-loop code, so the timeout must live
//! INSIDE the loop, not in an async select. The stream callbacks and the
//! format negotiation live in [`listeners`].

mod listeners;

use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use listeners::{enum_format_param, on_param_changed, on_process, on_state_changed};
use pipewire::context::ContextBox;
use pipewire::keys::{MEDIA_CATEGORY, MEDIA_ROLE, MEDIA_TYPE};
use pipewire::main_loop::MainLoopRc;
use pipewire::spa::param::video::VideoInfoRaw;
use pipewire::spa::pod::Pod;
use pipewire::spa::utils::Direction;
use pipewire::stream::{StreamBox, StreamFlags};

use super::error::PortalErrorKind;
use super::streams::MappedFormat;

/// The `PipeWire` stream name; also the node name visible in `pw-dump`
/// (the QA leak assert greps for it).
const STREAM_NAME: &str = "flowshot-portal";

/// One stream's first captured frame, in the orientation and format the
/// producer delivered (normalization happens in the screencast runner).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawPwFrame {
    /// The portal-assigned node id this frame arrived on.
    pub node_id: u32,
    /// Negotiated frame width in pixels.
    pub width: u32,
    /// Negotiated frame height in pixels.
    pub height: u32,
    /// Bytes per row (chunk-reported, may exceed `width * 4`).
    pub stride: u32,
    /// The negotiated format's frame mapping.
    pub format: MappedFormat,
    /// Raw pixel bytes (`chunk.size` bytes from `chunk.offset`).
    pub data: Vec<u8>,
}

/// Loop-shared capture state, behind one mutex: the callbacks run on the
/// loop thread and never await, so the guard is never held across a yield.
struct Shared {
    frames: Vec<RawPwFrame>,
    pending: usize,
    timed_out: bool,
    failure: Option<PortalErrorKind>,
}

/// Per-stream listener user data.
struct UserData {
    node_id: u32,
    format: VideoInfoRaw,
    shared: Arc<Mutex<Shared>>,
    main_loop: MainLoopRc,
}

fn lock_shared(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn pw_err(source: pipewire::Error) -> PortalErrorKind {
    PortalErrorKind::PipeWire { source }
}

/// Captures the first frame of every `nodes` stream through the portal's
/// `PipeWire` remote fd, bounded by `budget`.
///
/// # Errors
///
/// [`PortalErrorKind::NoStreams`] for an empty node list,
/// [`PortalErrorKind::PipeWire`] for loop/context/stream setup failures,
/// [`PortalErrorKind::Timeout`] when the budget expires before every stream
/// delivered a frame, plus the per-frame failures recorded by the callbacks
/// (`DmabufUnsupported`, `UnsupportedFormat`, `StreamFailed`,
/// `IncompleteFrame`).
pub(crate) fn capture_first_frames(
    fd: OwnedFd,
    nodes: &[u32],
    budget: Duration,
) -> Result<Vec<RawPwFrame>, PortalErrorKind> {
    if nodes.is_empty() {
        return Err(PortalErrorKind::NoStreams);
    }
    let main_loop = MainLoopRc::new(None).map_err(pw_err)?;
    let context = ContextBox::new(main_loop.loop_(), None).map_err(pw_err)?;
    let core = context.connect_fd(fd, None).map_err(pw_err)?;
    let shared = Arc::new(Mutex::new(Shared {
        frames: Vec::with_capacity(nodes.len()),
        pending: nodes.len(),
        timed_out: false,
        failure: None,
    }));

    let timer = main_loop.loop_().add_timer({
        let shared = Arc::clone(&shared);
        let main_loop = main_loop.clone();
        move |_| {
            lock_shared(&shared).timed_out = true;
            main_loop.quit();
        }
    });
    timer
        .update_timer(Some(budget), None)
        .into_sync_result()
        .map_err(|error| pw_err(error.into()))?;

    let format_param = enum_format_param()?;
    let mut streams = Vec::with_capacity(nodes.len());
    let mut listeners = Vec::with_capacity(nodes.len());
    for &node_id in nodes {
        let stream = StreamBox::new(
            &core,
            STREAM_NAME,
            pipewire::properties::properties! {
                *MEDIA_TYPE => "Video",
                *MEDIA_CATEGORY => "Capture",
                *MEDIA_ROLE => "Screen",
            },
        )
        .map_err(pw_err)?;
        let listener = stream
            .add_local_listener_with_user_data(UserData {
                node_id,
                format: VideoInfoRaw::default(),
                shared: Arc::clone(&shared),
                main_loop: main_loop.clone(),
            })
            .state_changed(on_state_changed)
            .param_changed(on_param_changed)
            .process(on_process)
            .register()
            .map_err(pw_err)?;
        let Some(pod) = Pod::from_bytes(&format_param) else {
            return Err(PortalErrorKind::Internal(
                "serialized format pod does not reparse",
            ));
        };
        let mut params = [pod];
        stream
            .connect(
                Direction::Input,
                Some(node_id),
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(pw_err)?;
        tracing::debug!(node_id, "portal screencast stream connected");
        listeners.push(listener);
        streams.push(stream);
    }

    main_loop.run();

    // Deterministic teardown: unregister callbacks, destroy the streams,
    // then disconnect the core (which removes this client's nodes from the
    // graph) before the context and loop die.
    drop(listeners);
    drop(streams);
    drop(timer);
    drop(core);
    drop(context);
    drop(main_loop);

    let mut shared = lock_shared(&shared);
    if shared.timed_out {
        return Err(PortalErrorKind::Timeout { timeout: budget });
    }
    if let Some(failure) = shared.failure.take() {
        return Err(failure);
    }
    if shared.frames.len() != nodes.len() {
        return Err(PortalErrorKind::IncompleteFrame);
    }
    Ok(std::mem::take(&mut shared.frames))
}
