//! The tier-2 technical payload: the GDPR-relevant details collected ONLY
//! with `[telemetry].include_technical_details` (full GPU adapter string,
//! exact kernel release, monitor layout, per-install UUID).
//!
//! Like the tier-1 taxonomy, every probe degrades to `None`/`"unknown"`
//! and never panics. The payload is computed ONCE at init (never
//! per-event), except the GPU adapter slot which a wgpu surface can
//! upgrade later through [`crate::telemetry::note_gpu_adapter`].

use std::path::Path;

/// The tier-2 payload, computed at init when the consent flag is set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Tier2Payload {
    /// Exact kernel release (`uname -r` equivalent, no libc: procfs).
    pub(crate) kernel_release: String,
    /// Connector + physical size + scale per output, comma-separated.
    /// `None` when the compositor probe failed (field omitted).
    pub(crate) monitors: Option<String>,
    /// The per-install random UUID (data-dir persisted, regenerable by
    /// deletion). `None` when it could neither be read nor generated.
    pub(crate) install_id: Option<String>,
    /// The `/sys` fallback GPU adapter string (PCI vendor:device +
    /// driver). The wgpu surfaces' [`crate::telemetry::note_gpu_adapter`]
    /// pass-through wins over it at event time.
    pub(crate) gpu_adapter: Option<String>,
}

impl Tier2Payload {
    /// Runs every tier-2 probe against the live system.
    pub(crate) fn probe(install_id_path: Option<&Path>) -> Self {
        Self {
            kernel_release: probe_kernel_release(),
            monitors: probe_monitor_layout(),
            install_id: install_id_path.and_then(install_id),
            gpu_adapter: probe_gpu_adapter(Path::new("/sys/class/drm")),
        }
    }
}

fn probe_kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The monitor layout through the tray's probe pattern: a dedicated
/// `CaptureThread` wayland connection, bounded by its internal deadlines,
/// shut down immediately (no resident connection).
fn probe_monitor_layout() -> Option<String> {
    use std::fmt::Write as _;
    let thread = flowshot_capture_wayland::CaptureThread::spawn().ok()?;
    let outputs = thread.outputs().ok();
    thread.shutdown();
    let outputs = outputs.filter(|outputs| !outputs.is_empty())?;
    let mut layout = String::new();
    for (index, output) in outputs.iter().enumerate() {
        if index > 0 {
            layout.push_str(", ");
        }
        // fmt::Write into a String cannot fail (it grows by allocation);
        // `let _` is the idiom for the infallible case.
        let _ = write!(
            layout,
            "{} {}x{}@{}",
            output.connector,
            output.physical_size.width.0,
            output.physical_size.height.0,
            output.scale
        );
    }
    Some(layout)
}

/// Reads the persisted install id, generating and persisting a fresh
/// random UUID v4 on first use (best-effort write: an unwritable data dir
/// still yields the generated id for this process).
fn install_id(path: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    let generated = random_uuid_v4()?;
    if let Some(parent) = path.parent() {
        // Best-effort persistence; a failure only means the id regenerates
        // next run (the file write is not worth failing telemetry over).
        if let Err(error) = std::fs::create_dir_all(parent)
            .and_then(|()| std::fs::write(path, format!("{generated}\n")))
        {
            tracing::debug!(%error, "telemetry install-id persistence failed");
        }
    }
    Some(generated)
}

/// 16 `/dev/urandom` bytes shaped into a UUID v4 (version + variant bits
/// per RFC 4122). No uuid crate: the workspace table does not carry one
/// for the daemon and the shape is ten lines.
fn random_uuid_v4() -> Option<String> {
    use std::fmt::Write as _;
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut uuid = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            uuid.push('-');
        }
        // Infallible String write (see probe_monitor_layout).
        let _ = write!(uuid, "{byte:02x}");
    }
    Some(uuid)
}

/// Pure GPU adapter string from one card's sysfs attributes:
/// `<driver> <vendor>:<device>` (PCI ids; the model NAME needs the wgpu
/// pass-through - sysfs does not carry it).
pub(crate) fn gpu_adapter_from_sysfs(vendor: &str, device: &str, driver: &str) -> String {
    format!(
        "{driver} {}:{}",
        vendor.trim_start_matches("0x"),
        device.trim_start_matches("0x")
    )
}

fn probe_gpu_adapter(drm_root: &Path) -> Option<String> {
    for card in super::environment::drm_cards(drm_root) {
        let device = card.join("device");
        let Some(driver) = std::fs::read_link(device.join("driver"))
            .ok()
            .and_then(|link| {
                link.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        else {
            continue;
        };
        let vendor = std::fs::read_to_string(device.join("vendor")).unwrap_or_default();
        let device_id = std::fs::read_to_string(device.join("device")).unwrap_or_default();
        if vendor.trim().is_empty() || device_id.trim().is_empty() {
            continue;
        }
        return Some(gpu_adapter_from_sysfs(
            vendor.trim(),
            device_id.trim(),
            &driver,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_release_probe_never_panics_and_yields_a_value() {
        let release = probe_kernel_release();
        assert_ne!(release, "");
    }

    #[test]
    fn uuid_v4_shape_carries_the_version_and_variant_bits() {
        let Some(uuid) = random_uuid_v4() else {
            return; // No /dev/urandom (non-Linux CI): nothing to assert.
        };
        let hex: String = uuid.chars().filter(|c| *c != '-').collect();
        assert_eq!(hex.len(), 32);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(uuid.chars().nth(14), Some('4'));
        assert!(matches!(uuid.chars().nth(19), Some('8' | '9' | 'a' | 'b')));
        // Two draws differ (randomness sanity).
        assert_ne!(Some(uuid), random_uuid_v4());
    }

    #[test]
    fn install_id_persists_and_rereads() {
        let dir =
            std::env::temp_dir().join(format!("flowshot-telemetry-test-{}", std::process::id()));
        let path = dir.join("telemetry-id");
        let first = install_id(&path);
        let second = install_id(&path);
        assert!(first.is_some());
        assert_eq!(first, second, "the persisted id must be stable");
        // Deletion regenerates (the documented user control).
        let _ = std::fs::remove_file(&path);
        let third = install_id(&path);
        assert!(third.is_some());
        assert_ne!(first, third);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpu_adapter_string_strips_the_hex_prefixes() {
        assert_eq!(
            gpu_adapter_from_sysfs("0x10de", "0x2204", "nvidia"),
            "nvidia 10de:2204"
        );
    }

    #[test]
    fn gpu_adapter_probe_degrades_on_a_missing_sysfs_root() {
        assert_eq!(probe_gpu_adapter(Path::new("/nonexistent-flowshot")), None);
    }

    #[test]
    fn monitor_layout_probe_degrades_without_a_compositor() {
        // Headless: neither WAYLAND_DISPLAY nor DISPLAY -> spawn fails ->
        // None, no panic. On a headed session (Wayland OR X11) the probe
        // returns the connector list or degrades to None (the probe drives
        // the Wayland capture stack only; X11 reports no monitor layout -
        // a documented degradation, see docs/setup-x11.md Known limits);
        // both are valid.
        let layout = probe_monitor_layout();
        let headed = ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
        if headed {
            assert!(layout.is_none_or(|text| !text.is_empty()));
        } else {
            assert_eq!(layout, None);
        }
    }
}
