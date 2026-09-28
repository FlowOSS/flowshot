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
