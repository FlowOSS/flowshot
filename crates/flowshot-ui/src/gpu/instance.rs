//! The wgpu instance policy: which backends every flowshot surface path
//! enumerates, and the explicit validation-flag choice.

/// Backends for the wgpu instance and adapter enumeration.
///
/// `PRIMARY` (Vulkan on Linux) deliberately excludes the GL/EGL backend: its
/// `eglTerminate` teardown segfaults inside NVIDIA's EGL-Wayland shim
/// (`wl_proxy_marshal_array_flags` on a freed proxy - live core dump
/// 2026-09-25) even when the chosen adapter is Vulkan, because the default
/// `InstanceDescriptor` initializes every backend eagerly. Vulkan covers the
/// plan's environments, including GPU-less ones via Mesa's software
/// rasterizer (lavapipe).
pub const OVERLAY_BACKENDS: wgpu::Backends = wgpu::Backends::PRIMARY;

/// Creates the wgpu instance for a flowshot GPU path (every window runtime
/// and QA harness): [`OVERLAY_BACKENDS`] plus the explicit validation-flag
/// choice below.
///
/// This is the single place the "`InstanceFlags::VALIDATION` requested, but
/// unable to find layer: `VK_LAYER_KHRONOS_validation`" line in debug-build
/// logs comes from: wgpu-hal warns when validation was requested and the
/// Khronos layer is not installed (benign - wgpu continues without it).
#[must_use]
pub fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: OVERLAY_BACKENDS,
        flags: instance_flags(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// Validation layer ON in debug builds, OFF in release.
///
/// Deliberately the exact `InstanceFlags::from_build_config()` value that
/// `InstanceDescriptor::new_without_display_handle()` would inherit silently,
/// spelled out so the debug-log validation warning is explained in code and
/// a release build can never drift into requesting the validation layer.
/// `VALIDATION_INDIRECT_CALL` (both profiles, per the wgpu 30 default) is
/// wgpu-internal indirect-draw argument checking - no layer involved, and
/// inert for flowshot's never-indirect draws.
#[must_use]
const fn instance_flags() -> wgpu::InstanceFlags {
    if cfg!(debug_assertions) {
        wgpu::InstanceFlags::DEBUG
            .union(wgpu::InstanceFlags::VALIDATION)
            .union(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL)
    } else {
        wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_match_the_wgpu_build_config_default() {
        // The explicit form must stay equivalent to what
        // `InstanceDescriptor::new_without_display_handle()` inherits, so the
        // switch to the explicit flags is behavior-preserving per build profile.
        assert_eq!(instance_flags(), wgpu::InstanceFlags::from_build_config());
        assert_eq!(
            instance_flags().contains(wgpu::InstanceFlags::VALIDATION),
            cfg!(debug_assertions),
            "the validation layer is requested in debug builds only"
        );
    }
}
