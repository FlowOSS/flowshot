//! Output enumeration: RANDR monitor data mapped onto the shared
//! [`OutputInfo`] geometry.
//!
//! The preferred source is RANDR >= 1.5 `GetMonitors` (active monitors - the
//! view `xrandr --listmonitors` shows); servers without it fall back to
//! enumerating lit CRTCs via `GetScreenResourcesCurrent` + `GetCrtcInfo` +
//! `GetOutputInfo`. Wire data is collected into the plain [`MonitorData`]
//! record first, so the mapping onto [`OutputInfo`] (scale derivation,
//! transform, logical rect) is unit-testable without a live X server - the
//! `OutputData` pattern of `flowshot-capture-wayland`.
//!
//! Physical-pixels-first (ADR-001): X screen coordinates *are* framebuffer
//! pixels (a root-window `GetImage` returns them 1:1), so the monitor rect
//! is physical truth and the logical rect is that rect divided by the
//! output's own derived scale. [`OutputInfo::physical_size`] carries the
//! pre-transform native size, so `buffer_size()` reproduces exactly the
//! screen-space extent capture covers. No scale is ever averaged across
//! outputs.

use flowshot_core::geometry::{
    GeometryError, OutputInfo, PhysicalPx, PhysicalRect, PhysicalSize, ToLogical, Transform,
};
use x11rb::protocol::randr::{self, Rotation, SetConfig};
use x11rb::protocol::xproto::{self, Atom, AtomEnum};
use x11rb::rust_connection::RustConnection;

use crate::connect::X11Connection;
use crate::error::X11Error;
use crate::scale::{derive_scale, parse_xft_dpi};

/// Read budget for the root `RESOURCE_MANAGER` property, in the 4-byte units
/// `GetProperty` counts (64 KiB of resource text - far above any realistic
/// `xrdb` database). A truncated read still finds `Xft.dpi` when it sits
/// inside the budget and degrades to the millimeter heuristic otherwise.
const RESOURCE_MANAGER_LONGS: u32 = 16 * 1024;

/// The four rotation bits inside a RANDR [`Rotation`] bitmask (the two
/// reflection bits sit above them).
const ROTATION_BITS: u16 = 0b1111;

/// Everything RANDR reported about one lit monitor, as plain data.
///
/// Positions and sizes are X screen space: post-transform framebuffer
/// pixels. The millimeter sizes are the panel-native values the server
/// copies from the output (EDID orientation, *not* adjusted for CRTC
/// rotation), which is why the scale heuristic in
/// [`MonitorData::to_output_info`] runs over the pre-transform pixel width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorData {
    /// Connector name (e.g. `eDP-1`), from the monitor name atom or the
    /// RANDR output name.
    pub connector: String,
    /// Screen-space position of the monitor's top-left corner.
    pub x: i16,
    /// Screen-space position of the monitor's top-left corner.
    pub y: i16,
    /// Screen-space width (post-transform framebuffer pixels).
    pub width: u16,
    /// Screen-space height (post-transform framebuffer pixels).
    pub height: u16,
    /// Panel-native physical width in millimeters (0 = not reported).
    pub mm_width: u32,
    /// Panel-native physical height in millimeters (0 = not reported).
    pub mm_height: u32,
    /// The CRTC's rotation/reflection bits.
    pub rotation: Rotation,
}

impl MonitorData {
    /// Assembles the shared [`OutputInfo`].
    ///
    /// `xft_dpi` is the session-global `Xft.dpi` resource when the desktop
    /// set one; the per-output millimeter heuristic is the fallback (plan
    /// decision #4, see [`crate::scale`]).
    ///
    /// # Errors
    ///
    /// Returns the [`GeometryError`] from [`OutputInfo::new`] when RANDR
    /// reported numbers the shared contract rejects; the caller skips such a
    /// monitor with a warning.
    pub(crate) fn to_output_info(&self, xft_dpi: Option<f64>) -> Result<OutputInfo, GeometryError> {
        let transform = transform_from_rotation(self.rotation);
        // The mm heuristic needs matching orientations: the server's mm
        // values are panel-native, so pair them with the pre-transform
        // pixel width (the screen width, swapped back for 90/270).
        let native_width = if transform.swaps_dimensions() {
            self.height
        } else {
            self.width
        };
        let scale = derive_scale(xft_dpi, u32::from(native_width), self.mm_width);
        let screen_rect = PhysicalRect::new(
            PhysicalPx(i32::from(self.x)),
            PhysicalPx(i32::from(self.y)),
            PhysicalPx(i32::from(self.width)),
            PhysicalPx(i32::from(self.height)),
        );
        let physical_size = transform.inverse().apply_to_size(PhysicalSize::new(
            PhysicalPx(i32::from(self.width)),
            PhysicalPx(i32::from(self.height)),
        ));
        OutputInfo::new(
            self.connector.clone(),
            self.connector.clone(),
            screen_rect.to_logical(scale),
            physical_size,
            scale,
            transform,
        )
    }
}

/// Maps RANDR rotation/reflection bits onto the shared [`Transform`].
///
/// RANDR reflects before it rotates - exactly the convention the Wayland
/// transform values use ("flipped around the vertical axis first, then
/// rotated counter-clockwise") - and `REFLECT_X` (negated x axis, a
/// left-right mirror) is the Wayland `Flipped`. `REFLECT_Y` (a top-bottom
/// mirror) equals `Flipped` plus a half turn, and both reflections together
/// cancel into a pure half turn; hence the `+2` quarter-turn offset and the
/// XOR below. This is the mapping mutter/muffin use for the same values.
///
/// Rotation bits outside the four defined values (a protocol violation) are
/// treated as unrotated.
pub(crate) fn transform_from_rotation(rotation: Rotation) -> Transform {
    let bits = u16::from(rotation);
    let reflect_x = bits & u16::from(Rotation::REFLECT_X) != 0;
    let reflect_y = bits & u16::from(Rotation::REFLECT_Y) != 0;
    let quarter_turns = match bits & ROTATION_BITS {
        b if b == u16::from(Rotation::ROTATE90) => 1,
        b if b == u16::from(Rotation::ROTATE180) => 2,
        b if b == u16::from(Rotation::ROTATE270) => 3,
        // ROTATE_0, or undefined bit combinations: treat as unrotated.
        _ => 0,
    };
    let flipped = reflect_x ^ reflect_y;
    let offset = if reflect_y { 2 } else { 0 };
    let index = (quarter_turns + offset) % 4 + if flipped { 4 } else { 0 };
    Transform::ALL[index]
}

/// One lit monitor as RANDR reported it, paired with the shared
/// [`OutputInfo`] assembled from it.
///
/// The capture path needs both views: [`MonitorData`] carries the screen-space
/// pixel rect `GetImage` reads (physical truth, never round-tripped through
/// the f64 logical rect), and [`OutputInfo`] carries the derived scale,
/// transform, and logical placement the [`Frame`](flowshot_capture::Frame)
/// contract requires.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MonitoredOutput {
    /// The raw RANDR monitor record (screen-space geometry).
    pub data: MonitorData,
    /// The shared output description derived from it.
    pub info: OutputInfo,
}

/// Enumerates the session's lit monitors with their derived [`OutputInfo`].
///
/// Tries RANDR `GetMonitors` first and falls back to lit-CRTC enumeration
/// when the server does not support it (RANDR 1.2-1.4). Monitors whose
/// numbers the shared geometry contract rejects - or that cover no pixels at
/// all (a zero-extent monitor has nothing to capture and `GetImage` would
/// reject it) - are skipped with a warning.
///
/// # Errors
///
/// Returns [`X11Error::Protocol`] when a RANDR or core request fails on the
/// live connection (on both the `GetMonitors` path and the CRTC fallback),
/// and propagates the [`X11Error`] of the `RESOURCE_MANAGER` read.
pub(crate) fn enumerate(conn: &X11Connection) -> Result<Vec<MonitoredOutput>, X11Error> {
    let xft_dpi = read_xft_dpi(conn)?;
    let monitors = match active_monitors(conn) {
        Ok(monitors) => monitors,
        Err(error) => {
            tracing::debug!(%error, "RANDR GetMonitors failed; falling back to CRTC enumeration");
            crtc_monitors(conn)?
        }
    };
    let mut paired = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        if monitor.width == 0 || monitor.height == 0 {
            tracing::warn!(
                connector = %monitor.connector,
                "skipping zero-extent X11 monitor; there is nothing to capture"
            );
            continue;
        }
        match monitor.to_output_info(xft_dpi) {
            Ok(info) => paired.push(MonitoredOutput {
                data: monitor,
                info,
            }),
            Err(error) => tracing::warn!(
                connector = %monitor.connector,
                %error,
                "skipping X11 output whose geometry the shared contract rejects"
            ),
        }
    }
    Ok(paired)
}

/// Enumerates the session's outputs with derived scale and transform.
///
/// The [`OutputInfo`] view of [`enumerate`]: one entry per lit monitor the
/// shared geometry contract accepts, in screen enumeration order.
///
/// # Errors
///
/// Propagates every error of [`enumerate`].
pub fn outputs(conn: &X11Connection) -> Result<Vec<OutputInfo>, X11Error> {
    Ok(enumerate(conn)?
        .into_iter()
        .map(|monitor| monitor.info)
        .collect())
}

/// Reads the session-global `Xft.dpi` from the root `RESOURCE_MANAGER`
/// property, when the desktop set one.
fn read_xft_dpi(conn: &X11Connection) -> Result<Option<f64>, X11Error> {
    let reply = xproto::get_property(
        conn.conn(),
        false,
        conn.root(),
        AtomEnum::RESOURCE_MANAGER,
        AtomEnum::ANY,
        0,
        RESOURCE_MANAGER_LONGS,
    )?
    .reply()?;
    if reply.format != 8 {
        // Absent (type NONE, empty value) or not text: no DPI statement.
        return Ok(None);
    }
    if reply.bytes_after > 0 {
        tracing::debug!(
            bytes_after = reply.bytes_after,
            "RESOURCE_MANAGER exceeds the read budget; Xft.dpi may be missed"
        );
    }
    Ok(parse_xft_dpi(&String::from_utf8_lossy(&reply.value)))
}

/// The RANDR >= 1.5 view: active monitors, one per lit output on stock
/// window managers. Rotation is not part of `GetMonitors`, so it is fetched
/// from the CRTC driving each monitor's first output.
fn active_monitors(conn: &X11Connection) -> Result<Vec<MonitorData>, X11Error> {
    let reply = randr::get_monitors(conn.conn(), conn.root(), true)?.reply()?;
    let mut monitors = Vec::with_capacity(reply.monitors.len());
    for monitor in reply.monitors {
        let connector = atom_name(conn.conn(), monitor.name)?;
        let rotation = output_rotation(
            conn.conn(),
            monitor.outputs.first().copied(),
            reply.timestamp,
        )?;
        monitors.push(MonitorData {
            connector,
            x: monitor.x,
            y: monitor.y,
            width: monitor.width,
            height: monitor.height,
            mm_width: monitor.width_in_millimeters,
            mm_height: monitor.height_in_millimeters,
            rotation,
        });
    }
    Ok(monitors)
}

/// The rotation of the CRTC currently driving `output`, or unrotated when
/// the monitor lists no output, the output is not on a CRTC, or a reply
/// reports a failed config status.
fn output_rotation(
    conn: &RustConnection,
    output: Option<randr::Output>,
    timestamp: xproto::Timestamp,
) -> Result<Rotation, X11Error> {
    let Some(output) = output else {
        return Ok(Rotation::ROTATE0);
    };
    let info = randr::get_output_info(conn, output, timestamp)?.reply()?;
    if info.status != SetConfig::SUCCESS || info.crtc == x11rb::NONE {
        return Ok(Rotation::ROTATE0);
    }
    let crtc = randr::get_crtc_info(conn, info.crtc, info.timestamp)?.reply()?;
    if crtc.status != SetConfig::SUCCESS {
        return Ok(Rotation::ROTATE0);
    }
    Ok(crtc.rotation)
}

/// The pre-1.5 fallback: every lit CRTC becomes one monitor record, named
/// and sized by its first output.
fn crtc_monitors(conn: &X11Connection) -> Result<Vec<MonitorData>, X11Error> {
    let resources = randr::get_screen_resources_current(conn.conn(), conn.root())?.reply()?;
    let mut monitors = Vec::new();
    for crtc_id in resources.crtcs {
        let crtc = randr::get_crtc_info(conn.conn(), crtc_id, resources.timestamp)?.reply()?;
        if crtc.status != SetConfig::SUCCESS
            || crtc.mode == x11rb::NONE
            || crtc.width == 0
            || crtc.height == 0
        {
            continue; // CRTC is off: nothing to capture.
        }
        let Some(&output_id) = crtc.outputs.first() else {
            continue; // Lit CRTC without outputs: nothing to name or size.
        };
        let output = randr::get_output_info(conn.conn(), output_id, crtc.timestamp)?.reply()?;
        if output.status != SetConfig::SUCCESS {
            continue;
        }
        monitors.push(MonitorData {
            connector: String::from_utf8_lossy(&output.name).into_owned(),
            x: crtc.x,
            y: crtc.y,
            width: crtc.width,
            height: crtc.height,
            mm_width: output.mm_width,
            mm_height: output.mm_height,
            rotation: crtc.rotation,
        });
    }
    Ok(monitors)
}

/// Resolves a monitor's name atom to its connector string.
fn atom_name(conn: &RustConnection, atom: Atom) -> Result<String, X11Error> {
    let reply = xproto::get_atom_name(conn, atom)?.reply()?;
    Ok(String::from_utf8_lossy(&reply.name).into_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixture scales are quantized 0.25 steps - exact binary fractions -
    // and fixture geometry is integral, so `assert_eq!` on f64 is the
    // intended assertion.
    #![allow(clippy::float_cmp)]

    use super::*;

    /// This machine's panel as RANDR reports it: eDP-1, 2880x1620 screen
    /// pixels over 344x194 mm, unrotated, at the screen origin.
    fn edp1() -> MonitorData {
        MonitorData {
            connector: "eDP-1".to_owned(),
            x: 0,
            y: 0,
            width: 2880,
            height: 1620,
            mm_width: 344,
            mm_height: 194,
            rotation: Rotation::ROTATE0,
        }
    }

    #[test]
    fn rotation_bits_map_onto_the_shared_transforms() {
        // The full defined 6-bit space: rotations, single reflections, and
        // both reflections (which cancel into a half turn).
        let cases = [
            (Rotation::ROTATE0, Transform::Normal),
            (Rotation::ROTATE90, Transform::Rot90),
            (Rotation::ROTATE180, Transform::Rot180),
            (Rotation::ROTATE270, Transform::Rot270),
            // REFLECT_X is the left-right mirror = Wayland Flipped.
            (Rotation::REFLECT_X, Transform::Flipped),
            (
                Rotation::REFLECT_X | Rotation::ROTATE90,
                Transform::Flipped90,
            ),
            (
                Rotation::REFLECT_X | Rotation::ROTATE180,
                Transform::Flipped180,
            ),
            (
                Rotation::REFLECT_X | Rotation::ROTATE270,
                Transform::Flipped270,
            ),
            // REFLECT_Y is the top-bottom mirror = Flipped + half turn.
            (Rotation::REFLECT_Y, Transform::Flipped180),
            (
                Rotation::REFLECT_Y | Rotation::ROTATE90,
                Transform::Flipped270,
            ),
            (
                Rotation::REFLECT_Y | Rotation::ROTATE180,
                Transform::Flipped,
            ),
            (
                Rotation::REFLECT_Y | Rotation::ROTATE270,
                Transform::Flipped90,
            ),
            // Both reflections = a pure half turn, composed with rotation.
            (Rotation::REFLECT_X | Rotation::REFLECT_Y, Transform::Rot180),
            (
                Rotation::REFLECT_X | Rotation::REFLECT_Y | Rotation::ROTATE90,
                Transform::Rot270,
            ),
            (
                Rotation::REFLECT_X | Rotation::REFLECT_Y | Rotation::ROTATE180,
                Transform::Normal,
            ),
            (
                Rotation::REFLECT_X | Rotation::REFLECT_Y | Rotation::ROTATE270,
                Transform::Rot90,
            ),
        ];
        for (rotation, expected) in cases {
            assert_eq!(
                transform_from_rotation(rotation),
                expected,
                "bits {rotation:?}"
            );
        }
    }

    #[test]
    fn undefined_rotation_bits_fall_back_to_normal() {
        assert_eq!(
            transform_from_rotation(Rotation::from(0u16)),
            Transform::Normal
        );
        assert_eq!(
            transform_from_rotation(Rotation::from(ROTATION_BITS)),
            Transform::Normal
        );
    }

    #[test]
    fn xft_dpi_drives_scale_and_logical_rect() {
        let info = edp1().to_output_info(Some(192.0)).unwrap();
        assert_eq!(info.connector, "eDP-1");
        assert_eq!(info.name, "eDP-1");
        assert_eq!(info.scale, 2.0);
        assert_eq!(info.physical_size.width.0, 2880);
        assert_eq!(info.physical_size.height.0, 1620);
        assert_eq!(info.logical_rect.x.0, 0.0);
        assert_eq!(info.logical_rect.width.0, 1440.0);
        assert_eq!(info.logical_rect.height.0, 810.0);
        assert_eq!(info.transform, Transform::Normal);
    }

    #[test]
    fn without_xft_dpi_the_mm_heuristic_derives_the_scale() {
        // 2880 px over 344 mm is ~212.7 dpi ~ 2.215, quantized to 2.25.
        let info = edp1().to_output_info(None).unwrap();
        assert_eq!(info.scale, 2.25);
        assert_eq!(info.logical_rect.width.0, 1280.0);
        assert_eq!(info.logical_rect.height.0, 720.0);
    }

    #[test]
    fn zero_mm_size_falls_back_to_scale_one_without_nan() {
        let data = MonitorData {
            mm_width: 0,
            mm_height: 0,
            ..edp1()
        };
        let info = data.to_output_info(None).unwrap();
        assert_eq!(info.scale, 1.0);
        assert!(info.scale.is_finite());
        assert_eq!(info.logical_rect.width.0, 2880.0);
    }

    #[test]
    fn rotated_monitor_reports_pre_transform_physical_size() {
        // A 2880x1620 panel rotated 90 degrees: the screen sees 1620x2880,
        // the native buffer stays 2880x1620, and buffer_size() re-derives
        // the screen-space extent capture covers.
        let data = MonitorData {
            width: 1620,
            height: 2880,
            rotation: Rotation::ROTATE90,
            ..edp1()
        };
        let info = data.to_output_info(Some(96.0)).unwrap();
        assert_eq!(info.transform, Transform::Rot90);
        assert_eq!(info.physical_size.width.0, 2880);
        assert_eq!(info.physical_size.height.0, 1620);
        assert_eq!(info.buffer_size().width.0, 1620);
        assert_eq!(info.buffer_size().height.0, 2880);
        assert_eq!(info.logical_rect.width.0, 1620.0);
    }

    #[test]
    fn rotated_monitor_mm_heuristic_uses_the_native_width() {
        // The server's mm values stay panel-native under rotation, so the
        // heuristic must pair 2880 native px (not the 1620 screen px) with
        // 344 mm - the same ~2.25 the unrotated panel derives.
        let data = MonitorData {
            width: 1620,
            height: 2880,
            rotation: Rotation::ROTATE90,
            ..edp1()
        };
        let info = data.to_output_info(None).unwrap();
        assert_eq!(info.scale, 2.25);
    }

    #[test]
    fn negative_screen_position_maps_into_the_logical_rect() {
        // Monitors left of the primary carry negative screen coordinates.
        let data = MonitorData {
            connector: "DP-1".to_owned(),
            x: -1920,
            width: 1920,
            height: 1080,
            mm_width: 508,
            mm_height: 286,
            ..edp1()
        };
        let info = data.to_output_info(Some(96.0)).unwrap();
        assert_eq!(info.logical_rect.x.0, -1920.0);
        assert_eq!(info.logical_rect.width.0, 1920.0);
    }
}
