//! Minimal desktop environment sniffing for the capability probe.
//!
//! This is deliberately a shallow environment-variable sniff: the full
//! layered desktop detection (with cursor-position capability tables) is a
//! later todo. `Other` is the honest fallback - never a guess.

use flowshot_capture::DesktopEnv;

/// Sniffs the desktop environment from the process environment.
///
/// Reads `XDG_CURRENT_DESKTOP` and `HYPRLAND_INSTANCE_SIGNATURE` once; the
/// values never change within a session.
#[must_use]
pub fn detect_desktop_env() -> DesktopEnv {
    desktop_env(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
    )
}

/// Pure desktop decision from the two environment values.
///
/// `HYPRLAND_INSTANCE_SIGNATURE` wins when set and non-empty (Hyprland
/// exports it unconditionally, while `XDG_CURRENT_DESKTOP` can be spoofed or
/// missing under systemd user services). Otherwise `XDG_CURRENT_DESKTOP` is
/// parsed as the colon-separated preference list the spec defines, matching
/// each entry exactly (case-insensitively) against the known desktops.
#[must_use]
pub fn desktop_env(
    xdg_current_desktop: Option<&str>,
    hyprland_instance_signature: Option<&str>,
) -> DesktopEnv {
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
            "gnome" => return DesktopEnv::Gnome,
            _ => {}
        }
    }
    DesktopEnv::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_signature_wins_over_any_desktop_var() {
        assert_eq!(
            desktop_env(Some("GNOME"), Some("1758e0f0_1758_0")),
            DesktopEnv::Hyprland
        );
    }

    #[test]
    fn empty_hyprland_signature_falls_through() {
        assert_eq!(desktop_env(Some("sway"), Some("")), DesktopEnv::Sway);
    }

    #[test]
    fn colon_separated_list_matches_any_entry() {
        assert_eq!(desktop_env(Some("ubuntu:GNOME"), None), DesktopEnv::Gnome);
        assert_eq!(desktop_env(Some("KDE"), None), DesktopEnv::Kde);
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
            assert_eq!(desktop_env(Some(value), None), expected, "{value}");
        }
    }

    #[test]
    fn unknown_or_missing_desktop_is_other() {
        assert_eq!(desktop_env(None, None), DesktopEnv::Other);
        assert_eq!(desktop_env(Some("i3"), None), DesktopEnv::Other);
        assert_eq!(desktop_env(Some(""), None), DesktopEnv::Other);
    }
}
