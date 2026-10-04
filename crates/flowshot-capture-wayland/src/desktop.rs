//! Desktop environment detection for the capability probe.
//!
//! A pure environment sniff in the Flameshot `desktopinfo` precedent (draft
//! F8): `WAYLAND_DISPLAY` decides whether this is a Wayland session at all,
//! `HYPRLAND_INSTANCE_SIGNATURE` pins Hyprland, and `XDG_CURRENT_DESKTOP` is
//! parsed as the colon-separated preference list the XDG spec defines.
//! [`DesktopEnv::Other`] is the honest fallback - never a guess.
//!
//! This crate is Wayland-only, so a process without `WAYLAND_DISPLAY` (X11,
//! a TTY, a foreign systemd service) detects as `Other` regardless of the
//! desktop name: there is no Wayland session for these backends to serve,
//! and claiming e.g. `Gnome` from an X11 GNOME session would misrepresent
//! that. The shared [`DesktopEnv`] vocabulary itself is session-agnostic; an
//! X11 platform crate (roadmap) grows its own detector.

use flowshot_capture::DesktopEnv;

/// Sniffs the desktop environment from the process environment.
///
/// Reads `XDG_CURRENT_DESKTOP`, `WAYLAND_DISPLAY`, and
/// `HYPRLAND_INSTANCE_SIGNATURE` once; the values never change within a
/// session.
#[must_use]
pub fn detect_desktop_env() -> DesktopEnv {
    desktop_env(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
    )
}

/// Pure desktop decision from the three environment values.
///
/// Decision order:
///
/// 1. No `WAYLAND_DISPLAY` (unset or empty): `Other` - not a Wayland
///    session, so no Wayland desktop claim is honest (module docs).
/// 2. `HYPRLAND_INSTANCE_SIGNATURE` set and non-empty: `Hyprland`.
///    Hyprland exports it unconditionally, while `XDG_CURRENT_DESKTOP` can
///    be spoofed or missing under systemd user services.
/// 3. `XDG_CURRENT_DESKTOP` parsed as the colon-separated preference list
///    the spec defines, each entry matched exactly (case-insensitively,
///    trimmed) against the known desktops - including the GNOME session
///    flavours (`GNOME-XORG`, `GNOME-Wayland`, `GNOME-Classic`,
///    `GNOME-Flashback`): the desktop identity is GNOME whichever flavour
///    names it.
/// 4. Anything else: `Other`.
#[must_use]
pub fn desktop_env(
    xdg_current_desktop: Option<&str>,
    wayland_display: Option<&str>,
    hyprland_instance_signature: Option<&str>,
) -> DesktopEnv {
    if wayland_display.is_none_or(str::is_empty) {
        return DesktopEnv::Other;
    }
    if hyprland_instance_signature.is_some_and(|signature| !signature.is_empty()) {
        return DesktopEnv::Hyprland;
    }
    let Some(desktops) = xdg_current_desktop else {
        return DesktopEnv::Other;
    };
    for desktop in desktops.split(':') {
        match desktop.trim().to_ascii_lowercase().as_str() {
            "hyprland" => return DesktopEnv::Hyprland,
            "sway" => return DesktopEnv::Sway,
            "niri" => return DesktopEnv::Niri,
            "cosmic" => return DesktopEnv::Cosmic,
            "kde" => return DesktopEnv::Kde,
            "gnome" | "gnome-wayland" | "gnome-xorg" | "gnome-classic" | "gnome-flashback" => {
                return DesktopEnv::Gnome;
            }
            _ => {}
        }
    }
    DesktopEnv::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live Wayland session's display value (the gate input).
    const DISPLAY: Option<&str> = Some("wayland-1");

    #[test]
    fn hyprland_signature_wins_over_any_desktop_var() {
        assert_eq!(
            desktop_env(Some("GNOME"), DISPLAY, Some("1758e0f0_1758_0")),
            DesktopEnv::Hyprland
        );
    }

    #[test]
    fn empty_hyprland_signature_falls_through() {
        assert_eq!(
            desktop_env(Some("sway"), DISPLAY, Some("")),
            DesktopEnv::Sway
        );
    }

    #[test]
    fn no_wayland_session_is_always_other() {
        // The gate precedes every other signal: a desktop name or a stale
        // signature without WAYLAND_DISPLAY describes no Wayland session.
        assert_eq!(desktop_env(Some("GNOME"), None, None), DesktopEnv::Other);
        assert_eq!(desktop_env(Some("KDE"), Some(""), None), DesktopEnv::Other);
        assert_eq!(
            desktop_env(Some("Hyprland"), None, Some("1758e0f0_1758_0")),
            DesktopEnv::Other
        );
    }

    #[test]
    fn colon_separated_list_matches_any_entry() {
        assert_eq!(
            desktop_env(Some("ubuntu:GNOME"), DISPLAY, None),
            DesktopEnv::Gnome
        );
        assert_eq!(desktop_env(Some("KDE"), DISPLAY, None), DesktopEnv::Kde);
    }

    #[test]
    fn every_known_desktop_maps_to_its_variant() {
        for (value, expected) in [
            ("Hyprland", DesktopEnv::Hyprland),
            ("sway", DesktopEnv::Sway),
            ("niri", DesktopEnv::Niri),
            ("COSMIC", DesktopEnv::Cosmic),
            ("kde", DesktopEnv::Kde),
            ("gnome", DesktopEnv::Gnome),
        ] {
            assert_eq!(desktop_env(Some(value), DISPLAY, None), expected, "{value}");
        }
    }

    #[test]
    fn gnome_session_flavours_map_to_gnome() {
        for value in [
            "GNOME-XORG",
            "gnome-xorg",
            "GNOME-Wayland",
            "GNOME-Classic",
            "GNOME-Flashback",
            "ubuntu:GNOME-XORG",
        ] {
            assert_eq!(
                desktop_env(Some(value), DISPLAY, None),
                DesktopEnv::Gnome,
                "{value}"
            );
        }
    }

    #[test]
    fn xorg_flavour_without_a_wayland_display_is_other() {
        // GNOME on Xorg exports no WAYLAND_DISPLAY: the Wayland detector
        // honestly reports Other instead of claiming an unusable session.
        assert_eq!(
            desktop_env(Some("GNOME-XORG"), None, None),
            DesktopEnv::Other
        );
    }

    #[test]
    fn unknown_or_missing_desktop_is_other() {
        assert_eq!(desktop_env(None, DISPLAY, None), DesktopEnv::Other);
        assert_eq!(desktop_env(Some("i3"), DISPLAY, None), DesktopEnv::Other);
        assert_eq!(desktop_env(Some(""), DISPLAY, None), DesktopEnv::Other);
        assert_eq!(
            desktop_env(Some("garbage:not-a-desktop"), DISPLAY, None),
            DesktopEnv::Other
        );
    }

    #[test]
    fn matching_is_case_insensitive_and_trims_whitespace() {
        assert_eq!(desktop_env(Some(" Sway "), DISPLAY, None), DesktopEnv::Sway);
        assert_eq!(
            desktop_env(Some("HYPRLAND"), DISPLAY, None),
            DesktopEnv::Hyprland
        );
    }
}
