//! Capture session state: registry globals, tracked outputs, and the
//! startup collection sequence that turns a fresh connection into a
//! [`SessionSnapshot`].

use std::collections::BTreeMap;

use flowshot_capture::{CapabilityProbe, DesktopEnv};
use flowshot_core::geometry::OutputInfo;
use serde::Serialize;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1;
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::cursor::protocol::{ActiveCursor, CursorSink};
use crate::desktop::detect_desktop_env;
use crate::error::ProbeError;
use crate::globals::{
    EXT_IMAGE_COPY_CAPTURE_MANAGER, EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER, Global,
    ProtocolGlobals, WL_OUTPUT, WL_SEAT, WL_SHM, WLR_SCREENCOPY_MANAGER, ZXDG_OUTPUT_MANAGER,
};
use crate::icc::protocol::ActiveCapture;
use crate::output::OutputData;
use crate::screencopy::protocol::ActiveScreencopy;

/// Highest `wl_output` version this crate binds (version 4 adds the `name`
/// and `description` events).
const WL_OUTPUT_VERSION: u32 = 4;
/// Highest `zxdg_output_manager_v1` version this crate binds (version 3 of
/// the manager hands out version-3 `zxdg_output_v1` objects).
const ZXDG_OUTPUT_MANAGER_VERSION: u32 = 3;
/// The `ext-image-copy-capture-v1` manager version this crate binds (the
/// protocol only has version 1).
const ICC_MANAGER_VERSION: u32 = 1;
/// The `wl_shm` version this crate binds (version 1 suffices for pools and
/// buffers).
const WL_SHM_VERSION: u32 = 1;
/// The `wl_seat` version this crate binds. Version 5 introduced the
/// `release` request; the pointer capability and `get_pointer` used by the
/// cursor session are stable since version 1, so a conservative cap avoids
/// requesting events this crate does not consume.
const WL_SEAT_VERSION: u32 = 5;
/// The `wlr-screencopy-unstable-v1` manager version this crate binds. Version
/// 3 adds the `buffer_done` event, the constraint-complete signal the v1
/// backend waits on (plan todo 9: v3 only, no v1/v2 paths).
const WLR_SCREENCOPY_MANAGER_VERSION: u32 = 3;

/// User data attached to output-scoped proxies (`wl_output`,
/// `zxdg_output_v1`): the registry name they were bound under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct OutputKey(pub u32);

/// One tracked output: its live proxies plus the plain event data.
#[derive(Debug)]
pub(crate) struct TrackedOutput {
    /// Kept alive for the session; also the handle capture backends need to
    /// create per-output capture sources.
    pub proxy: WlOutput,
    /// The `zxdg_output_v1` object for this output, when the compositor
    /// offers `xdg-output`.
    pub xdg: Option<ZxdgOutputV1>,
    /// The plain event data assembled into [`OutputInfo`] on snapshot.
    pub data: OutputData,
}

/// Everything one session probe observed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSnapshot {
    /// The capture-relevant protocol globals, with advertised versions.
    pub protocols: ProtocolGlobals,
    /// The backend-negotiation probe derived from `protocols` plus the
    /// desktop sniff.
    pub probe: CapabilityProbe,
    /// The enumerated outputs, in registry-name order.
    pub outputs: Vec<OutputInfo>,
}

/// The dispatch state of the capture thread's event queue.
#[derive(Debug)]
pub(crate) struct CaptureState {
    /// The bound registry. Kept alive so the compositor keeps reporting
    /// global additions and removals (hotplug) for the whole session.
    pub registry: Option<WlRegistry>,
    /// Every global the registry advertised, in arrival order.
    pub globals: Vec<Global>,
    /// Tracked outputs keyed by registry name (ordered for stable snapshots).
    pub outputs: BTreeMap<u32, TrackedOutput>,
    /// The bound `xdg-output` manager, when advertised.
    pub xdg_output_manager: Option<ZxdgOutputManagerV1>,
    /// The bound `wl_shm`, when advertised (capture buffer allocation).
    pub shm: Option<WlShm>,
    /// The bound `ext-image-copy-capture-v1` manager, when advertised.
    pub icc_manager: Option<ExtImageCopyCaptureManagerV1>,
    /// The bound per-output capture source manager, when advertised.
    pub icc_source_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    /// The bound `wlr-screencopy-unstable-v1` manager, when advertised.
    pub screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    /// The bound `wl_seat`, when advertised (pointer capability gates the
    /// cursor session).
    pub seat: Option<WlSeat>,
    /// Whether the bound seat advertised the pointer capability.
    pub seat_has_pointer: bool,
    /// The `wl_pointer` obtained from the seat, created on demand by
    /// [`CaptureState::ensure_pointer`] for cursor sessions.
    pub pointer: Option<WlPointer>,
    /// The event sink of the capture currently in flight (one-shot capture
    /// connections only; idle on the long-lived probe thread).
    pub active: ActiveCapture,
    /// The event sink of the screencopy frame in flight (one-shot screencopy
    /// capture connections only; idle otherwise).
    pub screencopy: ActiveScreencopy,
    /// The event sink of the cursor sessions in flight (one-shot cursor
    /// queries and the long-lived cursor stream; idle otherwise).
    pub cursor: ActiveCursor,
    /// Ordered output geometry the cursor sessions map positions against,
    /// filled before cursor sessions are created.
    pub cursor_layout: Vec<OutputInfo>,
    /// When set, cursor session events are forwarded here as they arrive
    /// (the long-lived cursor stream); one-shot queries leave it `None` and
    /// read [`CaptureState::cursor`] instead.
    pub cursor_sink: Option<CursorSink>,
    /// Set while a deadline-bounded display round-trip awaits its callback.
    pub roundtrip_pending: bool,
    /// Desktop environment sniffed once at session start.
    pub desktop: DesktopEnv,
}

impl CaptureState {
    /// Creates an empty state, sniffing the desktop environment.
    pub(crate) fn new() -> Self {
        Self {
            registry: None,
            globals: Vec::new(),
            outputs: BTreeMap::new(),
            xdg_output_manager: None,
            shm: None,
            icc_manager: None,
            icc_source_manager: None,
            screencopy_manager: None,
            seat: None,
            seat_has_pointer: false,
            pointer: None,
            active: ActiveCapture::default(),
            screencopy: ActiveScreencopy::default(),
            cursor: ActiveCursor::default(),
            cursor_layout: Vec::new(),
            cursor_sink: None,
            roundtrip_pending: false,
            desktop: detect_desktop_env(),
        }
    }

    /// Records one advertised global, binding the interfaces output
    /// enumeration and the capture backends need. Binding a manager is
    /// inert: capture sessions are created per capture run, not here.
    pub(crate) fn record_global(
        &mut self,
        registry: &WlRegistry,
        global: Global,
        qh: &QueueHandle<Self>,
    ) {
        tracing::debug!(
            name = global.name,
            interface = %global.interface,
            version = global.version,
            "registry global advertised"
        );
        match global.interface.as_str() {
            WL_OUTPUT => self.bind_output(registry, &global, qh),
            ZXDG_OUTPUT_MANAGER => {
                let manager: ZxdgOutputManagerV1 = registry.bind(
                    global.name,
                    global.version.min(ZXDG_OUTPUT_MANAGER_VERSION),
                    qh,
                    (),
                );
                self.xdg_output_manager = Some(manager);
            }
            WL_SHM => {
                let shm: WlShm =
                    registry.bind(global.name, global.version.min(WL_SHM_VERSION), qh, ());
                self.shm = Some(shm);
            }
            EXT_IMAGE_COPY_CAPTURE_MANAGER => {
                let manager: ExtImageCopyCaptureManagerV1 =
                    registry.bind(global.name, global.version.min(ICC_MANAGER_VERSION), qh, ());
                self.icc_manager = Some(manager);
            }
            EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER => {
                let manager: ExtOutputImageCaptureSourceManagerV1 =
                    registry.bind(global.name, global.version.min(ICC_MANAGER_VERSION), qh, ());
                self.icc_source_manager = Some(manager);
            }
            WLR_SCREENCOPY_MANAGER => {
                let manager: ZwlrScreencopyManagerV1 = registry.bind(
                    global.name,
                    global.version.min(WLR_SCREENCOPY_MANAGER_VERSION),
                    qh,
                    (),
                );
                self.screencopy_manager = Some(manager);
            }
            WL_SEAT => {
                let seat: WlSeat =
                    registry.bind(global.name, global.version.min(WL_SEAT_VERSION), qh, ());
                self.seat = Some(seat);
            }
            _ => {}
        }
        self.globals.push(global);
    }

    /// Drops a removed global and, when it was an output, its tracking.
    pub(crate) fn remove_global(&mut self, name: u32) {
        tracing::debug!(name, "registry global removed");
        self.globals.retain(|global| global.name != name);
        self.outputs.remove(&name);
    }

    /// Binds one `wl_output` and starts tracking it.
    fn bind_output(&mut self, registry: &WlRegistry, global: &Global, qh: &QueueHandle<Self>) {
        let proxy: WlOutput = registry.bind(
            global.name,
            global.version.min(WL_OUTPUT_VERSION),
            qh,
            OutputKey(global.name),
        );
        self.outputs.insert(
            global.name,
            TrackedOutput {
                proxy,
                xdg: None,
                data: OutputData::new(global.name),
            },
        );
        self.attach_xdg_output(global.name, qh);
    }

    /// Creates the `zxdg_output_v1` for one tracked output when the manager
    /// is bound and the output does not have one yet.
    fn attach_xdg_output(&mut self, name: u32, qh: &QueueHandle<Self>) {
        let Some(manager) = self.xdg_output_manager.clone() else {
            return;
        };
        let Some(output) = self.outputs.get(&name) else {
            return;
        };
        if output.xdg.is_some() {
            return;
        }
        let xdg: ZxdgOutputV1 = manager.get_xdg_output(&output.proxy, qh, OutputKey(name));
        if let Some(output) = self.outputs.get_mut(&name) {
            output.xdg = Some(xdg);
        }
    }

    /// Attaches `zxdg_output_v1` objects to every tracked output that lacks
    /// one (startup pass: `wl_output` globals can be advertised before the
    /// manager global arrives).
    pub(crate) fn attach_all_xdg_outputs(&mut self, qh: &QueueHandle<Self>) {
        for name in self.outputs.keys().copied().collect::<Vec<_>>() {
            self.attach_xdg_output(name, qh);
        }
    }

    /// Obtains the seat's `wl_pointer` when the seat advertised the pointer
    /// capability, caching it on the state. Cursor sessions need a pointer
    /// object; frame captures never call this, so they bind no pointer.
    ///
    /// Returns the pointer, or `None` when no seat was advertised or the seat
    /// lacks the pointer capability (the cursor path degrades to `None`).
    pub(crate) fn ensure_pointer(&mut self, qh: &QueueHandle<Self>) -> Option<WlPointer> {
        if let Some(pointer) = &self.pointer {
            return Some(pointer.clone());
        }
        if !self.seat_has_pointer {
            return None;
        }
        let seat = self.seat.clone()?;
        let pointer: WlPointer = seat.get_pointer(qh, ());
        self.pointer = Some(pointer.clone());
        Some(pointer)
    }

    /// Assembles the current snapshot, skipping outputs whose reported
    /// numbers fail geometry validation (warned, never fatal).
    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        let protocols = ProtocolGlobals::from_globals(&self.globals);
        let probe = protocols.to_capability_probe(self.desktop);
        let outputs = self
            .outputs
            .values()
            .filter_map(|tracked| {
                tracked
                    .data
                    .to_output_info()
                    .inspect_err(|err| {
                        tracing::warn!(
                            output = tracked.data.registry_name,
                            %err,
                            "skipping output with invalid reported geometry"
                        );
                    })
                    .ok()
            })
            .collect();
        SessionSnapshot {
            protocols,
            probe,
            outputs,
        }
    }
}

/// Runs the startup collection: registry listing, output binds, and the
/// `wl_output` + `zxdg_output_v1` event bursts.
///
/// Three synchronous round-trips: (1) the registry listing - its `global`
/// events bind every output and the `xdg-output` manager; (2) the
/// `wl_output` and `zxdg_output_v1` event bursts for those binds; (3) a
/// trailing round-trip so late `done` notifications cannot leak past
/// startup. Must run BEFORE the queue is handed to the event loop.
///
/// # Errors
///
/// Returns [`ProbeError::Dispatch`] when the compositor breaks the protocol
/// or the connection dies mid-collection.
pub(crate) fn collect_session(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
) -> Result<(), ProbeError> {
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    state.registry = Some(registry);
    queue.roundtrip(state)?;
    state.attach_all_xdg_outputs(&qh);
    queue.roundtrip(state)?;
    queue.roundtrip(state)?;
    Ok(())
}
