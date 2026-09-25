//! Typed errors for the overlay runtime.
//!
//! Every failure mode of startup, window creation, GPU initialization, and
//! frame presentation is a structured variant; no stringly-typed errors cross
//! crate boundaries (engineering standard, Amendment #4).

use flowshot_core::geometry::GeometryError;
use thiserror::Error;

/// Errors produced by the `FlowShot` UI runtime.
#[derive(Debug, Error)]
pub enum UiError {
    /// No display-server session was detected before startup.
    ///
    /// `FlowShot` is Wayland-native and never falls back to X11 (draft F9), so
    /// the runtime refuses to start without a compositor session.
    #[error(
        "no display server session detected: WAYLAND_DISPLAY is unset or empty. \
         Start FlowShot from inside a running Wayland compositor session \
         (FlowShot is Wayland-native and never falls back to X11)."
    )]
    NoDisplayServer,

    /// The compositor reported zero monitors.
    #[error("compositor reported no monitors; cannot spawn overlay windows")]
    NoMonitors,

    /// The winit event loop could not be created or exited with an error.
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),

    /// An event was sent to an event loop that already exited.
    #[error("overlay event loop has already exited; event discarded")]
    EventLoopClosed,

    /// A per-monitor overlay window could not be created.
    #[error("window creation failed on monitor \"{monitor}\": {source}")]
    WindowCreation {
        /// Name of the monitor the window was requested on.
        monitor: String,
        /// Underlying OS error.
        #[source]
        source: winit::error::OsError,
    },

    /// A monitor reported geometry that fails core validation.
    #[error("monitor \"{name}\" reports invalid geometry: {source}")]
    MonitorGeometry {
        /// Name of the offending monitor.
        name: String,
        /// Underlying geometry validation error.
        #[source]
        source: GeometryError,
    },

    /// A monitor reported a physical size that exceeds `i32` bounds.
    #[error("monitor \"{name}\" reports an unrepresentable size: {width}x{height} physical px")]
    MonitorSizeOverflow {
        /// Name of the offending monitor.
        name: String,
        /// Reported width in physical pixels.
        width: u32,
        /// Reported height in physical pixels.
        height: u32,
    },

    /// No GPU adapter was enumerated at all (no graphics backend produced
    /// one). Distinct from [`UiError::NoQualifiedAdapter`]: here the list of
    /// adapters is empty, not merely too weak.
    #[error("no GPU adapter was enumerated; no graphics backend is available")]
    NoGpuAdapter,

    /// GPU adapters exist but none can drive the overlay: every one misses
    /// the `max_texture_dimension_2d` floor or cannot present to the probe
    /// surface. The message lists every enumerated adapter with its cap so
    /// the failure is diagnosable without a GPU debugger.
    #[error(
        "no GPU adapter qualifies for the overlay (requires max_texture_dimension_2d >= \
         {required} and surface support); enumerated adapters: {}",
        format_adapter_reports(adapters)
    )]
    NoQualifiedAdapter {
        /// The `max_texture_dimension_2d` floor every adapter missed.
        required: u32,
        /// Every enumerated adapter with its texture cap and surface support.
        adapters: Vec<GpuAdapterReport>,
    },

    /// The GPU device could not be created.
    #[error("GPU device creation failed: {0}")]
    GpuDevice(#[from] wgpu::RequestDeviceError),

    /// A surface extent exceeds the device's maximum 2D texture dimension.
    ///
    /// Checked BEFORE `wgpu::Surface::configure`, which panics on oversized
    /// extents; v1 is all-or-nothing, so one monitor failing this check tears
    /// the whole overlay down with this typed error (Amendment #4: no panics).
    #[error(
        "surface for monitor \"{monitor}\" needs {width}x{height} px but the GPU device \
         limits 2D textures to {max} px per dimension"
    )]
    SurfaceSizeExceedsLimits {
        /// Name of the monitor whose surface does not fit.
        monitor: String,
        /// Requested surface width in physical pixels.
        width: u32,
        /// Requested surface height in physical pixels.
        height: u32,
        /// The device's `max_texture_dimension_2d`.
        max: u32,
    },

    /// A wgpu surface could not be created for a window.
    #[error("surface creation failed for monitor \"{monitor}\": {source}")]
    SurfaceCreation {
        /// Name of the monitor whose surface failed.
        monitor: String,
        /// Underlying wgpu error.
        #[source]
        source: wgpu::CreateSurfaceError,
    },

    /// A surface advertised no usable texture formats.
    #[error("surface for monitor \"{monitor}\" supports no texture formats")]
    NoSurfaceFormats {
        /// Name of the monitor whose surface is unusable.
        monitor: String,
    },

    /// Presentation ran out of memory.
    #[error("GPU out of memory while presenting a frame")]
    OutOfMemory,

    /// A geometry operation inside the input router failed validation.
    #[error("geometry error: {0}")]
    Geometry(#[from] GeometryError),
}

/// One GPU adapter enumerated at startup, reported inside
/// [`UiError::NoQualifiedAdapter`] so a limits failure names every candidate
/// and its texture cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdapterReport {
    /// Driver-reported adapter name.
    pub name: String,
    /// The adapter's `max_texture_dimension_2d` limit.
    pub max_texture_dimension_2d: u32,
    /// Whether the adapter can present to the overlay's probe surface.
    pub surface_supported: bool,
}

impl std::fmt::Display for GpuAdapterReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (max_texture_dimension_2d={}, surface {})",
            self.name,
            self.max_texture_dimension_2d,
            if self.surface_supported {
                "supported"
            } else {
                "unsupported"
            }
        )
    }
}

fn format_adapter_reports(reports: &[GpuAdapterReport]) -> String {
    reports
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_qualified_adapter_message_lists_every_adapter_name_and_cap() {
        let error = UiError::NoQualifiedAdapter {
            required: 4096,
            adapters: vec![
                GpuAdapterReport {
                    name: "llvmpipe".to_owned(),
                    max_texture_dimension_2d: 2048,
                    surface_supported: true,
                },
                GpuAdapterReport {
                    name: "AMD Radeon RX 6800".to_owned(),
                    max_texture_dimension_2d: 16384,
                    surface_supported: false,
                },
            ],
        };
        let message = error.to_string();
        for token in ["llvmpipe", "2048", "AMD Radeon RX 6800", "16384", "4096"] {
            assert!(message.contains(token), "missing {token:?} in: {message}");
        }
    }
}
