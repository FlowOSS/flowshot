//! Stub-driven integration tests for the `ScreenShot2` chain: a private
//! `zbus` peer serves the pinned wire contract (see [`stub`] for the
//! upstream citations) and the production run functions drive it end to
//! end - `D-Bus` call, fd-IN pipe, raw payload readback, frame assembly.
//!
//! Every test here holds [`STUB_LOCK`]: the stubs' sockets and pipes live
//! in the process-wide descriptor table, and the fd-leak assertions compare
//! `/proc/self/fd` snapshots that parallel stub tests would otherwise
//! contaminate.
//!
//! [`stub`]: crate::kwin::stub

#![allow(clippy::unwrap_used, clippy::expect_used)]
// Fixture scales and sizes are exact integral literals (geometry-notepad
// convention), so strict comparison is the intended assertion.
#![allow(clippy::float_cmp)]

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use flowshot_capture::{BackendKind, FrameFormat, OutputRef, negotiate};
use flowshot_core::geometry::{
    Logical, LogicalRect, OutputInfo, PhysicalPx, PhysicalSize, Transform,
};

use super::error::{DecodeError, KwinError};
use super::probe::probe_on;
use super::run::{run_selected, run_single};
use super::stub::{StubFixture, spawn_stub};
use super::wire::{KwinBus, Request};

static STUB_LOCK: Mutex<()> = Mutex::new(());

fn stub_guard() -> MutexGuard<'static, ()> {
    STUB_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn output(connector: &str, physical: (i32, i32), scale: f64, transform: Transform) -> OutputInfo {
    let swaps = matches!(
        transform,
        Transform::Rot90 | Transform::Rot270 | Transform::Flipped90 | Transform::Flipped270
    );
    let (logical_width, logical_height) = if swaps {
        (physical.1, physical.0)
    } else {
        (physical.0, physical.1)
    };
    let (scale_x, scale_y) = (
        f64::from(logical_width) / scale,
        f64::from(logical_height) / scale,
    );
    OutputInfo::new(
        connector,
        connector,
        LogicalRect::new(
            Logical(0.0),
            Logical(0.0),
            Logical(scale_x),
            Logical(scale_y),
        ),
        PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
        scale,
        transform,
    )
    .unwrap()
}

/// The process's open descriptors mapped to their `/proc` link targets.
/// The snapshot's own directory handle (`/proc/self/fd`) is filtered out -
/// it is observer noise, closed deterministically when the scan returns.
fn fd_targets() -> BTreeMap<i32, String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let fd = entry.file_name().to_string_lossy().parse().ok()?;
            let target = std::fs::read_link(entry.path())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (!target.starts_with("/proc/")).then_some((fd, target))
        })
        .collect()
}

/// Descriptor entries opened since `baseline` that survive a bounded
/// quiescence window. Comparison is by (fd, target) PAIR, not fd number
/// alone: a closed baseline descriptor's number can be reused by a new
/// socket (masking it from a number-keyed diff), and a leaked descriptor
/// can land on a recycled number. `/proc/self/fd` is process-wide, so
/// parallel non-stub tests contribute transient entries - a genuine leak
/// never closes, transient noise does, hence the bounded re-scan.
fn persistent_new_fds(baseline: &BTreeMap<i32, String>) -> BTreeMap<i32, String> {
    let new_since = || {
        fd_targets()
            .into_iter()
            .filter(|(fd, target)| baseline.get(fd) != Some(target))
            .collect::<BTreeMap<i32, String>>()
    };
    let mut leaked = new_since();
    for _ in 0..10 {
        if leaked.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        leaked = new_since();
    }
    leaked
}

/// Polls `condition` for up to ~200ms. Every use bounds a teardown step
/// that is GUARANTEED to complete: `Connection::close` issues
/// `shutdown(Both)`, which resolves the zbus reader task's parked read
/// immediately (reactor wakeup, no peer dependency), and the task then
/// releases the socket descriptor.
fn wait_until(condition: impl Fn() -> bool) -> bool {
    for _ in 0..10 {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

/// The socket descriptors opened by the `spawn_stub` call that just ran
/// (exactly the p2p pair: server end + client end), as (fd, target) pairs
/// so a recycled fd number cannot impersonate them.
fn stub_pair_sockets(pre_stub: &BTreeMap<i32, String>) -> Vec<(i32, String)> {
    fd_targets()
        .into_iter()
        .filter(|(fd, target)| target.starts_with("socket:") && pre_stub.get(fd) != Some(target))
        .collect()
}

/// How many of the (fd, target) pairs are still open.
fn open_pairs(pairs: &[(i32, String)]) -> usize {
    let current = fd_targets();
    pairs
        .iter()
        .filter(|(fd, target)| current.get(fd) == Some(target))
        .count()
}

/// Pre-initializes tokio's process-global signal socketpair.
///
/// The FIRST `enable_all` runtime in a process creates this pair and tokio
/// deliberately keeps it in process-global signal state - it survives every
/// runtime drop and is reused by all later runtimes. Without this step, a
/// run whose runtime happens to be the process's first would place that
/// infrastructure pair inside the measurement window (this is why the test
/// passed in full-suite runs - another test's runtime won the race - and
/// failed in isolation). It is not a leak of the code under test.
fn preinit_tokio_global_signal_pair() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    drop(runtime);
}

#[test]
fn no_fd_leaks_on_success_or_failure() {
    let _guard = stub_guard();
    preinit_tokio_global_signal_pair();

    // Success leg: every descriptor that survives the run already existed
    // before it, AND the run's deterministic `Session` teardown closed the
    // client end of the stub pair (only the server end, held by the bus,
    // remains); dropping the bus then closes the server end too.
    let pre_stub = fd_targets();
    let mut bus = spawn_stub(StubFixture::argb32(4, 3), true, true);
    let stub_sockets = stub_pair_sockets(&pre_stub);
    assert_eq!(stub_sockets.len(), 2, "expected the p2p socket pair");
    let baseline = fd_targets();
    let client = bus.client_conn();
    let success = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    );
    bus.state.join_writers();
    assert!(success.is_ok());
    let leaked = persistent_new_fds(&baseline);
    assert!(leaked.is_empty(), "success path leaked fds {leaked:?}");
    assert!(
        wait_until(|| open_pairs(&stub_sockets) == 1),
        "the run must close the client socket; {} stub sockets still open",
        open_pairs(&stub_sockets)
    );
    drop(bus);
    assert!(
        wait_until(|| open_pairs(&stub_sockets) == 0),
        "dropping the bus must close the server socket; {} still open",
        open_pairs(&stub_sockets)
    );

    // Failure leg (truncated payload): the error path unwinds through the
    // same `Session` Drop, so both pipe ends and the client socket close
    // exactly as on the success path.
    let pre_stub = fd_targets();
    let mut bus = spawn_stub(
        StubFixture {
            truncate: 8,
            ..StubFixture::argb32(4, 3)
        },
        true,
        true,
    );
    let stub_sockets = stub_pair_sockets(&pre_stub);
    assert_eq!(stub_sockets.len(), 2, "expected the p2p socket pair");
    let baseline = fd_targets();
    let client = bus.client_conn();
    let failure = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    );
    bus.state.join_writers();
    assert!(matches!(
        failure,
        Err(KwinError::Decode(DecodeError::TruncatedPayload { .. }))
    ));
    let leaked = persistent_new_fds(&baseline);
    assert!(leaked.is_empty(), "failure path leaked fds {leaked:?}");
    assert!(
        wait_until(|| open_pairs(&stub_sockets) == 1),
        "the failed run must close the client socket; {} stub sockets still open",
        open_pairs(&stub_sockets)
    );
    drop(bus);
    assert!(
        wait_until(|| open_pairs(&stub_sockets) == 0),
        "dropping the bus must close the server socket; {} still open",
        open_pairs(&stub_sockets)
    );
}

#[test]
fn stub_round_trip_area_capture_matches_the_raw_fixture() {
    let _guard = stub_guard();
    // Given a stub serving a 4x3 ARGB32 raw fixture with additive keys,
    let fixture = StubFixture::argb32(4, 3);
    let payload = fixture.payload.clone();
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    // When the production chain captures one output over the private peer,
    let captured = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    )
    .unwrap();
    bus.state.join_writers();
    // Then the frame carries the fixture's exact raw bytes and geometry.
    assert_eq!(captured.frames.len(), 1);
    let frame = &captured.frames[0];
    assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
    assert_eq!(frame.buffer.stride, 16);
    assert_eq!(frame.buffer.format, FrameFormat::Argb8888);
    assert_eq!(frame.buffer.data.as_ref(), payload.as_slice());
    assert_eq!(frame.output, OutputRef::Connector("DP-1".to_owned()));
    assert_eq!(frame.transform, Transform::Normal);
    // And the stub saw CaptureArea with the logical rect and the options.
    let served = bus.state.served();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].member, "CaptureArea");
    assert_eq!(served[0].area, Some((0, 0, 4, 3)));
    assert_eq!(served[0].include_cursor, Some(true));
    assert_eq!(served[0].native_resolution, Some(true));
}

#[test]
fn rgbx_fixture_forces_opaque_alpha() {
    let _guard = stub_guard();
    let mut fixture = StubFixture::argb32(2, 1);
    fixture.format = 16;
    fixture.payload = vec![1, 2, 3, 0, 5, 6, 7, 0];
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    let captured = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (2, 1), 1.0, Transform::Normal)],
        true,
    )
    .unwrap();
    bus.state.join_writers();
    let buffer = &captured.frames[0].buffer;
    assert_eq!(buffer.format, FrameFormat::Rgba8888);
    assert_eq!(buffer.pixel(0, 0), Some([1, 2, 3, 255]));
    assert_eq!(buffer.pixel(1, 0), Some([5, 6, 7, 255]));
}

#[test]
fn large_payload_drains_through_pipe_backpressure() {
    let _guard = stub_guard();
    // 256x256 ARGB32 = 256 KiB, far beyond the 64 KiB pipe buffer: the
    // writer blocks until the client's read loop drains it (KWin's
    // poll-based backpressure semantics).
    let fixture = StubFixture::argb32(256, 256);
    let expected_len = fixture.payload.len();
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    let captured = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (256, 256), 1.0, Transform::Normal)],
        true,
    )
    .unwrap();
    bus.state.join_writers();
    assert_eq!(captured.frames[0].buffer.data.len(), expected_len);
}

#[test]
fn rotated_output_area_capture_inverse_remaps_to_native() {
    let _guard = stub_guard();
    // Given a Rot90 output (native 3x4, upright 4x3) whose upright fixture
    // has a red BOTTOM row,
    let mut fixture = StubFixture::argb32(4, 3);
    fixture.payload = [[0u8, 255, 0, 255].repeat(8), [255u8, 0, 0, 255].repeat(4)].concat();
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    // When capturing through the production chain,
    let captured = run_selected(
        KwinBus::Peer(client),
        vec![output("R", (3, 4), 1.0, Transform::Rot90)],
        true,
    )
    .unwrap();
    bus.state.join_writers();
    // Then the frame is native 3x4 with the red row as the LEFT column,
    // carrying the transform as metadata.
    let frame = &captured.frames[0];
    assert_eq!((frame.buffer.width, frame.buffer.height), (3, 4));
    assert_eq!(frame.transform, Transform::Rot90);
    for y in 0..4 {
        assert_eq!(frame.buffer.pixel(0, y), Some([255, 0, 0, 255]));
        assert_eq!(frame.buffer.pixel(1, y), Some([0, 255, 0, 255]));
    }
}

#[test]
fn truncated_payload_is_a_typed_decode_error() {
    let _guard = stub_guard();
    let mut fixture = StubFixture::argb32(4, 3);
    fixture.truncate = 8;
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    let result = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    );
    bus.state.join_writers();
    match result.unwrap_err() {
        KwinError::Decode(DecodeError::TruncatedPayload { expected, received }) => {
            assert_eq!(expected, 48);
            assert_eq!(received, 40);
        }
        other => panic!("expected TruncatedPayload, got {other:?}"),
    }
}

#[test]
fn unknown_qimage_format_is_a_typed_decode_error() {
    let _guard = stub_guard();
    let mut fixture = StubFixture::argb32(4, 3);
    fixture.format = 999;
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    let result = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    );
    bus.state.join_writers();
    match result.unwrap_err() {
        KwinError::Decode(DecodeError::UnknownFormat { format }) => assert_eq!(format, 999),
        other => panic!("expected UnknownFormat, got {other:?}"),
    }
}

#[test]
fn dimension_mismatch_against_the_output_is_typed() {
    let _guard = stub_guard();
    let mut bus = spawn_stub(StubFixture::argb32(8, 8), true, true);
    let client = bus.client_conn();
    let result = run_selected(
        KwinBus::Peer(client),
        vec![output("DP-1", (4, 3), 1.0, Transform::Normal)],
        true,
    );
    bus.state.join_writers();
    match result.unwrap_err() {
        KwinError::BufferSizeMismatch { reported, expected } => {
            assert_eq!(reported, (8, 8));
            assert_eq!(expected, (4, 3));
        }
        other => panic!("expected BufferSizeMismatch, got {other:?}"),
    }
}

#[test]
fn single_image_capture_returns_an_upright_composite_frame() {
    let _guard = stub_guard();
    let mut fixture = StubFixture::argb32(4, 3);
    fixture.scale = 2.0;
    let payload = fixture.payload.clone();
    let mut bus = spawn_stub(fixture, true, true);
    let client = bus.client_conn();
    let frame = run_single(KwinBus::Peer(client), Request::ActiveScreen, false).unwrap();
    bus.state.join_writers();
    assert_eq!(frame.output, OutputRef::Composite);
    assert_eq!(frame.transform, Transform::Normal);
    assert_eq!(frame.scale, 2.0);
    assert_eq!((frame.buffer.width, frame.buffer.height), (4, 3));
    assert_eq!(frame.buffer.data.as_ref(), payload.as_slice());
    let served = bus.state.served();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].member, "CaptureActiveScreen");
    assert_eq!(served[0].include_cursor, Some(false));
}

#[test]
fn probe_includes_the_backend_when_the_name_is_owned_and_introspection_answers() {
    let _guard = stub_guard();
    let mut bus = spawn_stub(StubFixture::argb32(1, 1), true, true);
    let connection = bus.client_conn();
    let availability = futures::executor::block_on(probe_on(&connection));
    assert!(availability.screenshot2);
    let mut probe = flowshot_capture::CapabilityProbe::new(
        flowshot_capture::DesktopEnv::Kde,
        [BackendKind::PortalScreenshot],
    );
    availability.observe_into(&mut probe);
    let ladder = negotiate(&probe, None).unwrap();
    assert_eq!(
        ladder,
        vec![BackendKind::KwinScreenShot2, BackendKind::PortalScreenshot]
    );
    drop(bus);
}

#[test]
fn probe_excludes_the_backend_when_the_name_owner_is_absent() {
    let _guard = stub_guard();
    let mut bus = spawn_stub(StubFixture::argb32(1, 1), false, true);
    let connection = bus.client_conn();
    let availability = futures::executor::block_on(probe_on(&connection));
    assert!(!availability.screenshot2);
    let mut probe = flowshot_capture::CapabilityProbe::new(
        flowshot_capture::DesktopEnv::Kde,
        [BackendKind::PortalScreenshot],
    );
    availability.observe_into(&mut probe);
    assert_eq!(
        negotiate(&probe, None).unwrap(),
        vec![BackendKind::PortalScreenshot]
    );
    drop(bus);
}

#[test]
fn probe_excludes_the_backend_when_introspection_fails() {
    let _guard = stub_guard();
    // The name is owned but no ScreenShot2 object answers Properties.Get.
    let mut bus = spawn_stub(StubFixture::argb32(1, 1), true, false);
    let connection = bus.client_conn();
    let availability = futures::executor::block_on(probe_on(&connection));
    assert!(!availability.screenshot2);
    drop(bus);
}
