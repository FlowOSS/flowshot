//! GPU adapter selection policy and device-limits floor.
//!
//! The design requires 4K headroom (the capture backend delivers 3840x2160
//! buffers and the renderer paints 4K frames), and the live 2-monitor session
//! panicked when downlevel
//! defaults capped textures at 2048 px against a 2560x1440 output (issues.md
//! 2026-09-25 "downlevel wgpu limits = live-only bug class"). The policy here
//! is pure data - candidate snapshots instead of live `wgpu::Adapter`s - so
//! selection and the size floor are unit-testable without a GPU.

use crate::error::GpuAdapterReport;

/// Hard floor for `max_texture_dimension_2d` on the chosen adapter and device.
///
/// 4096 spans every 4K output (3840x2160) the plan targets; adapters capped
/// below it (downlevel/WebGL-class 2048) are rejected at selection, and the
/// device is requested with the chosen adapter's own limits - never the
/// 2048-capped downlevel defaults.
pub(crate) const MIN_TEXTURE_DIMENSION_2D: u32 = 4096;

/// Power-preference classes for adapter selection, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerClass {
    /// Discrete GPU (the `wgpu::PowerPreference::HighPerformance` class).
    HighPerformance,
    /// Integrated GPU (the `wgpu::PowerPreference::LowPower` class).
    LowPower,
    /// Software, virtual, or unknown device types.
    Other,
}

impl PowerClass {
    /// Selection order: `HighPerformance`, then `LowPower`, then any
    /// qualifying candidate.
    const PREFERENCE_ORDER: [Self; 3] = [Self::HighPerformance, Self::LowPower, Self::Other];

    pub(crate) const fn from_device_type(device_type: wgpu::DeviceType) -> Self {
        match device_type {
            wgpu::DeviceType::DiscreteGpu => Self::HighPerformance,
            wgpu::DeviceType::IntegratedGpu => Self::LowPower,
            wgpu::DeviceType::Other | wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Cpu => {
                Self::Other
            }
        }
    }
}

/// GPU-free snapshot of one enumerated adapter: everything [`select_adapter`]
/// needs. Tests build these with fake [`wgpu::Limits`] structs.
#[derive(Debug, Clone)]
pub(crate) struct AdapterCandidate {
    /// Driver-reported adapter name.
    pub name: String,
    /// Power class derived from the adapter's device type.
    pub power: PowerClass,
    /// Whether the adapter can present to the overlay's probe surface.
    pub surface_supported: bool,
    /// The adapter's full supported limits.
    pub limits: wgpu::Limits,
}

impl AdapterCandidate {
    /// Whether this candidate meets the texture floor and can present.
    fn qualifies(&self) -> bool {
        self.surface_supported && self.limits.max_texture_dimension_2d >= MIN_TEXTURE_DIMENSION_2D
    }

    /// The report embedded in [`UiError::NoQualifiedAdapter`](crate::UiError::NoQualifiedAdapter).
    pub(crate) fn report(&self) -> GpuAdapterReport {
        GpuAdapterReport {
            name: self.name.clone(),
            max_texture_dimension_2d: self.limits.max_texture_dimension_2d,
            surface_supported: self.surface_supported,
        }
    }
}

/// Pure adapter selection: the index of the first qualifying candidate in
/// power-preference order (`HighPerformance`, then `LowPower`, then any), or
/// `None` when no candidate meets the floor. Enumeration order breaks ties
/// within a class.
pub(crate) fn select_adapter(candidates: &[AdapterCandidate]) -> Option<usize> {
    PowerClass::PREFERENCE_ORDER.iter().find_map(|class| {
        candidates
            .iter()
            .position(|candidate| candidate.power == *class && candidate.qualifies())
    })
}

/// The GPU renderer's per-frame offscreen cost in bytes: the multisampled RGBA8
/// color target (4 bytes per sample) plus the `Stencil8` attachment (1 byte),
/// at the surface extent.
///
/// This is the allocation that made an OOM-starved desktop die at the swapchain
/// rather than at `request_device`: measured ~230 MB at the 4480x1440 dual
/// monitor union with 8x MSAA.
/// The MSAA rung the policy budgets: the upper bound of
/// `render::target::msaa_sample_count` (8 when the device has
/// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, else 4), so the policy
/// over-estimates the target rather than under-estimating it.
pub(crate) const MSAA_SAMPLES: u32 = 8;

/// The offscreen budget above which the CPU raster path takes over.
///
/// Set below the measured 230 MB of the dual-monitor union (which OOM'd) and
/// above the single 2560x1440 output (~74 MB, which presented fine), so the
/// policy splits the two observed cases rather than guessing between them.
pub(crate) const GPU_TARGET_MEMORY_BUDGET: usize = 128 << 20;

/// The GPU renderer's offscreen target bytes for a `width` x `height` extent.
#[must_use]
pub(crate) fn offscreen_target_bytes(width: u32, height: u32) -> usize {
    let pixels = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
    pixels * (usize::try_from(MSAA_SAMPLES).unwrap_or(0) * 4 + 1)
}

/// Whether the CPU raster path should present a `width` x `height` window
/// surface driven by a `power`-class adapter: a software/virtual/CPU driver
/// (whose `encode+submit` was measured at ~63 ms/frame) or an extent whose
/// offscreen target exceeds the memory budget.
#[must_use]
pub(crate) fn software_path_for(power: PowerClass, width: u32, height: u32) -> bool {
    match power {
        PowerClass::Other => true,
        PowerClass::HighPerformance | PowerClass::LowPower => {
            offscreen_target_bytes(width, height) > GPU_TARGET_MEMORY_BUDGET
        }
    }
}

/// Environment variable that masks the render-path policy (the QA harness for
/// the CPU raster fallback): `software` forces the CPU raster path at any
/// extent and adapter class, `gpu` forces the GPU path regardless of the
/// policy; anything else leaves the policy in charge. Byte-exact tokens, so a
/// typo stays visible as policy-driven behavior.
pub(crate) const RENDER_PATH_OVERRIDE_ENV: &str = "FLOWSHOT_RENDER_PATH";

/// QA override rule for the render path: `software` forces the CPU raster
/// path at any extent and adapter class, `gpu` forces the GPU path
/// regardless of the policy, and anything else (including unset) leaves the
/// policy in charge. Tokens are byte-exact so a typo in the environment shows
/// up as policy-driven behavior rather than a silently forced path.
#[must_use]
pub(crate) fn render_path_for_override(value: Option<&std::ffi::OsStr>) -> Option<bool> {
    if value == Some(std::ffi::OsStr::new("software")) || value == Some(std::ffi::OsStr::new("cpu"))
    {
        Some(true)
    } else if value == Some(std::ffi::OsStr::new("gpu")) {
        Some(false)
    } else {
        None
    }
}

/// The `FLOWSHOT_RENDER_PATH` environment read, split from the rule so the
/// rule stays unit-testable (this crate keeps policy pure data).
#[must_use]
pub(crate) fn render_path_override() -> Option<bool> {
    render_path_for_override(std::env::var_os(RENDER_PATH_OVERRIDE_ENV).as_deref())
}

/// Whether a `width` x `height` surface extent fits a device whose
/// `max_texture_dimension_2d` is `limit`.
///
/// Checked BEFORE every `wgpu::Surface::configure`: wgpu panics on oversized
/// configs, and this crate forbids panics - the caller converts `false`
/// into a typed [`UiError::SurfaceSizeExceedsLimits`](crate::UiError::SurfaceSizeExceedsLimits).
pub(crate) const fn surface_size_fits(width: u32, height: u32, limit: u32) -> bool {
    width <= limit && height <= limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_texture_dimension_2d: u32) -> wgpu::Limits {
        wgpu::Limits {
            max_texture_dimension_2d,
            ..wgpu::Limits::default()
        }
    }

    fn candidate(
        name: &str,
        power: PowerClass,
        surface_supported: bool,
        max_texture_dimension_2d: u32,
    ) -> AdapterCandidate {
        AdapterCandidate {
            name: name.to_owned(),
            power,
            surface_supported,
            limits: limits(max_texture_dimension_2d),
        }
    }

    #[test]
    fn surface_size_floor_table() {
        let cases = [
            // The live bug: DP-3 2560x1440 against the 2048 downlevel cap.
            (2048, 2560, 1440, false),
            // The capture backend's 4K buffers against the downlevel cap.
            (2048, 3840, 2160, false),
            // The required floor covers the live session.
            (4096, 2560, 1440, true),
            // 4K headroom above the floor.
            (8192, 3840, 2160, true),
            // Exactly at the cap fits; one pixel over does not.
            (2048, 2048, 1, true),
            (4096, 4097, 100, false),
        ];
        for (cap, width, height, fits) in cases {
            assert_eq!(
                surface_size_fits(width, height, cap),
                fits,
                "{width}x{height} against cap {cap}"
            );
        }
    }

    #[test]
    fn software_path_matches_the_measured_cases() {
        let cases = [
            // The OOM'd dual-monitor union: 4480x1440 with 8x MSAA + stencil
            // (213 MB), above the budget.
            (PowerClass::HighPerformance, 4480, 1440, true),
            // The single 2560x1440 output that presented fine (116 MB).
            (PowerClass::HighPerformance, 2560, 1440, false),
            // A 4K single output (274 MB) also takes the CPU path.
            (PowerClass::LowPower, 3840, 2160, true),
            // A software/virtual/CPU driver takes the CPU path at any extent.
            (PowerClass::Other, 1, 1, true),
            // The budget boundary: just under it stays on the GPU path,
            // just over takes the CPU path.
            (PowerClass::HighPerformance, 1, 4_000_000, false),
            (PowerClass::HighPerformance, 1, 4_100_000, true),
        ];
        for (power, width, height, software) in cases {
            assert_eq!(software_path_for(power, width, height), software);
        }
    }

    #[test]
    fn render_path_override_names_only_the_two_tokens() {
        let cases = [
            (Some(std::ffi::OsStr::new("software")), Some(true)),
            (Some(std::ffi::OsStr::new("gpu")), Some(false)),
            (None, None),
            (Some(std::ffi::OsStr::new("")), None),
            (Some(std::ffi::OsStr::new("SOFTWARE")), None),
            (Some(std::ffi::OsStr::new("software ")), None),
            (Some(std::ffi::OsStr::new("cpu")), Some(true)),
            (Some(std::ffi::OsStr::new("invalid")), None),
        ];
        for (value, expected) in cases {
            assert_eq!(render_path_for_override(value), expected, "token {value:?}");
        }
    }

    #[test]
    fn select_prefers_high_performance_then_low_power_then_other() {
        let candidates = [
            candidate("cpu-raster", PowerClass::Other, true, 8192),
            candidate("igpu", PowerClass::LowPower, true, 16384),
            candidate("dgpu", PowerClass::HighPerformance, true, 16384),
        ];
        assert_eq!(select_adapter(&candidates), Some(2));
        assert_eq!(select_adapter(&candidates[..2]), Some(1));
        assert_eq!(select_adapter(&candidates[..1]), Some(0));
    }

    #[test]
    fn select_skips_below_floor_and_surface_unsupported() {
        let candidates = [
            candidate("dgpu-downlevel", PowerClass::HighPerformance, true, 2048),
            candidate("igpu-headless", PowerClass::LowPower, false, 16384),
            candidate("software", PowerClass::Other, true, 4096),
        ];
        assert_eq!(select_adapter(&candidates), Some(2));
    }

    #[test]
    fn select_returns_none_when_nothing_qualifies() {
        let candidates = [
            candidate("dgpu-2048", PowerClass::HighPerformance, true, 2048),
            candidate("igpu-2048", PowerClass::LowPower, true, 2048),
        ];
        assert_eq!(select_adapter(&candidates), None);
        assert_eq!(select_adapter(&[]), None);
    }

    #[test]
    fn select_keeps_enumeration_order_within_a_class() {
        let candidates = [
            candidate("dgpu-first", PowerClass::HighPerformance, true, 8192),
            candidate("dgpu-second", PowerClass::HighPerformance, true, 16384),
        ];
        assert_eq!(select_adapter(&candidates), Some(0));
    }

    #[test]
    fn device_type_maps_to_power_class() {
        use wgpu::DeviceType;
        assert_eq!(
            PowerClass::from_device_type(DeviceType::DiscreteGpu),
            PowerClass::HighPerformance
        );
        assert_eq!(
            PowerClass::from_device_type(DeviceType::IntegratedGpu),
            PowerClass::LowPower
        );
        for other in [DeviceType::Cpu, DeviceType::VirtualGpu, DeviceType::Other] {
            assert_eq!(PowerClass::from_device_type(other), PowerClass::Other);
        }
    }

    #[test]
    fn report_carries_name_cap_and_surface_support() {
        let report = candidate("igpu", PowerClass::LowPower, false, 16384).report();
        assert_eq!(
            report,
            GpuAdapterReport {
                name: "igpu".to_owned(),
                max_texture_dimension_2d: 16384,
                surface_supported: false,
            }
        );
    }
}
