//! Tier-1 environment taxonomy: the coarse, non-identifying facts attached
//! to every event (distro, arch, package manager, session type, desktop
//! family, compositor version, GPU family).
//!
//! Every probe degrades to `"unknown"` (or `None` for the optional
//! compositor version) and NEVER panics: telemetry must not be able to
//! break a capture. The decision logic lives in pure functions that take
//! their inputs as parameters (tests never mutate process env or touch the
//! real filesystem - the paths.rs parallel-test convention); the `probe_*`
//! functions are the thin env/fs glue.
//!
//! # Probe order (documented per the telemetry spec)
//!
//! - distro: `/etc/os-release` `NAME=` (quoted values unwrapped).
//! - arch: [`std::env::consts::ARCH`].
//! - package manager: the flatpak marker `/run/.flatpak-info` FIRST (inside
//!   a flatpak the PATH describes the runtime, not the host, so host
//!   binaries are not probed), then PATH existence in the order
//!   `pacman` -> `dpkg` -> `apt` -> `nix`.
//! - session type: `XDG_SESSION_TYPE` (lowercased), else `WAYLAND_DISPLAY`
//!   presence -> `wayland`, else `DISPLAY` presence -> `x11`.
//! - desktop family: [`crate::shortcut::detect_desktop`] (the shared
//!   `DesktopEnv` vocabulary - the daemon-local portable probe, NOT the
//!   wayland crate's detector).
//! - compositor version: `hyprctl version` ONLY on Hyprland - a bounded
//!   `std::process` probe at INIT time (the daemon crate is the shell
//!   boundary; the gated crates never shell out). NEVER per-event.
//! - GPU family: `/sys/class/drm/card<N>/device/driver` symlink names,
//!   first card in sorted order with a driver wins; `nvidia` -> `nvidia`,
//!   `amdgpu`/`radeon` -> `amd`, `i915`/`xe` -> `intel`, any other driver
//!   -> `other`, no card/driver -> `unknown`.

use std::borrow::Cow;
use std::path::Path;

use flowshot_capture::DesktopEnv;

/// The flatpak runtime marker (present inside a flatpak sandbox).
const FLATPAK_MARKER: &str = "/run/.flatpak-info";

/// The package-manager binaries probed on PATH, in order.
const PACKAGE_MANAGER_PROBES: [&str; 4] = ["pacman", "dpkg", "apt", "nix"];

/// Budget for the init-time compositor-version shell-out.
const COMPOSITOR_PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// The tier-1 taxonomy, probed ONCE at init and attached to every event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Environment {
    pub(crate) distro: String,
    pub(crate) arch: &'static str,
    pub(crate) package_manager: String,
    pub(crate) session_type: String,
    pub(crate) desktop: String,
    /// `None` when not Hyprland or the probe failed (tag omitted).
    pub(crate) compositor_version: Option<String>,
    pub(crate) gpu_family: String,
}

impl Environment {
    /// Runs every probe against the live process environment.
    pub(crate) fn probe() -> Self {
        let desktop_env = crate::shortcut::detect_desktop();
        Self {
            distro: probe_distro(),
            arch: std::env::consts::ARCH,
            package_manager: probe_package_manager().to_owned(),
            session_type: probe_session_type().to_owned(),
            desktop: desktop_tag(desktop_env).to_owned(),
            compositor_version: match desktop_env {
                DesktopEnv::Hyprland => probe_hyprland_version(),
                DesktopEnv::Sway
                | DesktopEnv::Niri
                | DesktopEnv::Cosmic
                | DesktopEnv::Kde
                | DesktopEnv::Gnome
                | DesktopEnv::Other => None,
            },
            gpu_family: probe_gpu_family(Path::new("/sys/class/drm")).to_owned(),
        }
    }
}

/// The `DesktopEnv` -> tag-string mapping (the shared vocabulary's serde
/// spelling; `Other` is the honest fallback, never a guess).
pub(crate) const fn desktop_tag(desktop: DesktopEnv) -> &'static str {
    match desktop {
        DesktopEnv::Hyprland => "hyprland",
        DesktopEnv::Sway => "sway",
        DesktopEnv::Niri => "niri",
        DesktopEnv::Cosmic => "cosmic",
        DesktopEnv::Kde => "kde",
        DesktopEnv::Gnome => "gnome",
        DesktopEnv::Other => "other",
    }
}

/// Pure distro decision from the `/etc/os-release` text.
pub(crate) fn distro_from_os_release(text: &str) -> Cow<'_, str> {
    for line in text.lines() {
        let Some(value) = line.strip_prefix("NAME=") else {
            continue;
        };
        let trimmed = value.trim().trim_matches('"');
        if !trimmed.is_empty() {
            return Cow::Owned(trimmed.to_owned());
        }
    }
    Cow::Borrowed("unknown")
}

fn probe_distro() -> String {
    let Ok(text) = std::fs::read_to_string("/etc/os-release") else {
        return "unknown".to_owned();
    };
    distro_from_os_release(&text).into_owned()
}

/// Pure package-manager decision: the flatpak marker wins, then the PATH
/// probes in the documented order (the probe names ARE the tag values).
pub(crate) fn package_manager_from(flatpak: bool, on_path: impl Fn(&str) -> bool) -> &'static str {
    if flatpak {
        return "flatpak";
    }
    for name in PACKAGE_MANAGER_PROBES {
        if on_path(name) {
            return name;
        }
    }
    "unknown"
}

fn probe_package_manager() -> &'static str {
    let path = std::env::var_os("PATH").unwrap_or_default();
    package_manager_from(Path::new(FLATPAK_MARKER).exists(), |name| {
        std::env::split_paths(&path).any(|dir| binary_exists(&dir, name))
    })
}

fn binary_exists(dir: &Path, name: &str) -> bool {
    // An empty PATH component means the cwd; never probe it (a file named
    // `pacman` in the cwd is not a package manager).
    if dir.as_os_str().is_empty() {
        return false;
    }
    dir.join(name).is_file()
}

/// Pure session-type decision from the three environment values.
pub(crate) fn session_type_from(
    xdg_session_type: Option<&str>,
    wayland_display: Option<&str>,
    display: Option<&str>,
) -> &'static str {
    match xdg_session_type.map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) if value.eq_ignore_ascii_case("wayland") => "wayland",
        Some(value) if value.eq_ignore_ascii_case("x11") => "x11",
        Some(value) if value.eq_ignore_ascii_case("tty") => "tty",
        // An unrecognized XDG_SESSION_TYPE is reported verbatim-ish as
        // unknown: the taxonomy never invents a family.
        Some(_) => "unknown",
        None => {
            if wayland_display.is_some_and(|v| !v.is_empty()) {
                "wayland"
            } else if display.is_some_and(|v| !v.is_empty()) {
                "x11"
            } else {
                "unknown"
            }
        }
    }
}

fn probe_session_type() -> &'static str {
    session_type_from(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
    )
}

/// Pure `hyprctl version` output parse: the second token of the first
/// line (`Hyprland 0.56.2 built from ...`).
pub(crate) fn hyprland_version_from(output: &str) -> Option<String> {
    let first_line = output.lines().next()?;
    let mut tokens = first_line.split_whitespace();
    let head = tokens.next()?;
    let version = tokens.next()?;
    head.eq_ignore_ascii_case("hyprland")
        .then(|| version.to_owned())
}

/// The bounded init-time shell-out (the ONLY one in telemetry; documented
/// in the module header). Polls `try_wait` so a hung child is killed at the
/// budget instead of blocking init forever. The invocation is the
/// `version` SUBCOMMAND: Hyprland 0.56's hyprctl answers `--version` with
/// the usage text (the legacy-flag removal class from the notepad).
fn probe_hyprland_version() -> Option<String> {
    use std::process::Stdio;
    let mut child = std::process::Command::new("hyprctl")
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + COMPOSITOR_PROBE_BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    String::from_utf8(output.stdout)
        .ok()
        .as_deref()
        .and_then(hyprland_version_from)
}

/// Pure GPU-family decision from the DRM driver names of the cards, in
/// sorted card order (first card with a driver wins).
pub(crate) fn gpu_family_from_drivers<'a>(
    drivers: impl IntoIterator<Item = &'a str>,
) -> &'static str {
    let mut saw_driver = false;
    for driver in drivers {
        saw_driver = true;
        match driver {
            "nvidia" => return "nvidia",
            "amdgpu" | "radeon" => return "amd",
            "i915" | "xe" => return "intel",
            _ => {}
        }
    }
    if saw_driver { "other" } else { "unknown" }
}

fn probe_gpu_family(drm_root: &Path) -> &'static str {
    let drivers: Vec<String> = drm_cards(drm_root)
        .iter()
        .filter_map(|card| {
            std::fs::read_link(card.join("device").join("driver"))
                .ok()?
                .file_name()?
                .to_str()
                .map(str::to_owned)
        })
        .collect();
    gpu_family_from_drivers(drivers.iter().map(String::as_str))
}

/// The sorted `/sys/class/drm/card<N>` device paths (the connector
/// entries like `card0-DP-1` are excluded: only whole cards carry the
/// `device/driver` link).
pub(crate) fn drm_cards(drm_root: &Path) -> Vec<std::path::PathBuf> {
    let mut cards: Vec<_> = std::fs::read_dir(drm_root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("card"))
                .is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .collect();
    cards.sort();
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distro_parses_name_with_and_without_quotes() {
        assert_eq!(
            distro_from_os_release("NAME=\"Arch Linux\"\nID=arch\n"),
            "Arch Linux"
        );
        assert_eq!(distro_from_os_release("NAME=Fedora\n"), "Fedora");
        assert_eq!(distro_from_os_release("ID=arch\n"), "unknown");
        assert_eq!(distro_from_os_release("NAME=\n"), "unknown");
        assert_eq!(distro_from_os_release(""), "unknown");
        // PRETTY_NAME must not be mistaken for NAME (prefix match).
        assert_eq!(
            distro_from_os_release("PRETTY_NAME=\"Arch Linux\"\n"),
            "unknown"
        );
    }

    #[test]
    fn package_manager_probe_order_is_flatpak_then_pacman_dpkg_apt_nix() {
        assert_eq!(package_manager_from(true, |_| true), "flatpak");
        assert_eq!(package_manager_from(false, |_| false), "unknown");
        assert_eq!(
            package_manager_from(false, |name| name == "pacman" || name == "nix"),
            "pacman"
        );
        assert_eq!(
            package_manager_from(false, |name| name == "dpkg" || name == "apt"),
            "dpkg"
        );
        assert_eq!(package_manager_from(false, |name| name == "apt"), "apt");
        assert_eq!(package_manager_from(false, |name| name == "nix"), "nix");
    }

    #[test]
    fn session_type_prefers_xdg_then_display_vars() {
        assert_eq!(session_type_from(Some("wayland"), None, None), "wayland");
        assert_eq!(session_type_from(Some(" Wayland "), None, None), "wayland");
        assert_eq!(session_type_from(Some("x11"), None, None), "x11");
        assert_eq!(session_type_from(Some("tty"), None, None), "tty");
        assert_eq!(session_type_from(Some("mir"), None, None), "unknown");
        assert_eq!(
            session_type_from(Some(""), Some("wayland-1"), None),
            "wayland"
        );
        assert_eq!(session_type_from(None, None, Some(":0")), "x11");
        assert_eq!(session_type_from(None, Some(""), Some("")), "unknown");
        assert_eq!(session_type_from(None, None, None), "unknown");
    }

    #[test]
    fn hyprland_version_parses_the_first_line_second_token() {
        assert_eq!(
            hyprland_version_from("Hyprland 0.56.2 built from branch main at commit abc"),
            Some("0.56.2".to_owned())
        );
        assert_eq!(hyprland_version_from("hyprctl 0.1"), None);
        assert_eq!(hyprland_version_from("Hyprland"), None);
        assert_eq!(hyprland_version_from(""), None);
    }

    #[test]
    fn gpu_family_maps_known_drivers_and_degrades() {
        assert_eq!(gpu_family_from_drivers(["nvidia"]), "nvidia");
        assert_eq!(gpu_family_from_drivers(["amdgpu"]), "amd");
        assert_eq!(gpu_family_from_drivers(["radeon"]), "amd");
        assert_eq!(gpu_family_from_drivers(["i915"]), "intel");
        assert_eq!(gpu_family_from_drivers(["xe"]), "intel");
        assert_eq!(gpu_family_from_drivers(["vmwgfx"]), "other");
        assert_eq!(gpu_family_from_drivers([] as [&str; 0]), "unknown");
        // First card in order wins.
        assert_eq!(gpu_family_from_drivers(["i915", "nvidia"]), "intel");
    }

    #[test]
    fn gpu_family_probe_degrades_on_a_missing_sysfs_root() {
        assert_eq!(
            probe_gpu_family(Path::new("/nonexistent-flowshot-probe")),
            "unknown"
        );
    }

    #[test]
    fn every_probe_degrades_without_panic_on_this_machine() {
        // The live glue must never panic whatever the environment holds.
        let environment = Environment::probe();
        assert_ne!(environment.distro, "");
        assert_ne!(environment.package_manager, "");
        assert_ne!(environment.session_type, "");
        assert_ne!(environment.desktop, "");
        assert_ne!(environment.gpu_family, "");
        assert_eq!(environment.arch, std::env::consts::ARCH);
    }

    #[test]
    fn live_machine_reports_the_expected_taxonomy() {
        // The QA machines: an x86_64 Arch Hyprland Wayland box and an
        // x86_64 Arch i3 X11 box (Phase A). The exact-value asserts only
        // run when the live environment actually is one of those sessions
        // (CI containers degrade to unknown, which the test then verifies
        // instead).
        let environment = Environment::probe();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            assert_eq!(environment.session_type, "wayland");
            assert_eq!(environment.desktop, "hyprland");
            assert_eq!(environment.arch, "x86_64");
            assert_eq!(environment.distro, "Arch Linux");
            assert_eq!(environment.package_manager, "pacman");
            assert_eq!(environment.gpu_family, "nvidia");
            assert!(
                environment.compositor_version.is_some(),
                "hyprctl --version must resolve on a Hyprland session"
            );
        } else if std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()) {
            // A headed X11 session: the taxonomy names it; no wayland
            // desktop tag and no compositor version exist on X11.
            assert_eq!(environment.session_type, "x11");
            assert_eq!(environment.desktop, "other");
            assert_eq!(environment.compositor_version, None);
        } else {
            assert_eq!(environment.session_type, "unknown");
            assert_eq!(environment.desktop, "other");
            assert_eq!(environment.compositor_version, None);
        }
    }
}
