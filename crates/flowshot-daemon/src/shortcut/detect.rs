//! Desktop detection and the portal override for the shortcut ladder.
//!
//! Detection mirrors `flowshot-capture-wayland`'s decision order (shared
//! [`DesktopEnv`] vocabulary from `flowshot-capture`): the Wayland gate
//! first (the interactive surfaces this ladder feeds are Wayland-only, so
//! an X11 session must not claim a Wayland desktop), then
//! `HYPRLAND_INSTANCE_SIGNATURE` (exported
//! unconditionally by Hyprland, spoofable `XDG_CURRENT_DESKTOP` second),
//! then the colon-separated `XDG_CURRENT_DESKTOP` preference list.
//!
//! The pure function takes the environment values as parameters (tests
//! never mutate process env - parallel-test race avoidance); [`detect_desktop`]
//! is the thin env-reading glue.

use flowshot_capture::DesktopEnv;

/// Environment variable that masks the portal path (the QA "portal
/// masked (env override harness)" failure scenario): `0`/`off`/`false`/
/// `disabled` (case-insensitive) skip the portal attempt entirely and go
/// straight to the compositor-bind fallback.
pub const PORTAL_OVERRIDE_ENV: &str = "FLOWSHOT_SHORTCUTS_PORTAL";

/// Whether the portal registration attempt runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortalMode {
    /// Try the portal first, fall back to compositor snippets on failure
    /// (the shortcut ladder).
    #[default]
    Auto,
    /// Skip the portal (QA harness / user override via
    /// [`PORTAL_OVERRIDE_ENV`]).
    Disabled,
}

impl PortalMode {
    /// The pure override decision from the variable's value.
    #[must_use]
    pub fn from_override(value: Option<&str>) -> Self {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value)
                if ["0", "off", "false", "disabled"]
                    .contains(&value.to_ascii_lowercase().as_str()) =>
            {
                Self::Disabled
            }
            _ => Self::Auto,
        }
    }

    /// Reads [`PORTAL_OVERRIDE_ENV`].
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_override(std::env::var(PORTAL_OVERRIDE_ENV).ok().as_deref())
    }
}

/// The pure desktop decision from the two environment values (the
/// capture-wayland decision order minus its capture-specific concerns).
#[must_use]
pub fn desktop_from_env(
    wayland_display: Option<&str>,
    xdg_current_desktop: Option<&str>,
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

/// Detects the desktop from the process environment.
#[must_use]
pub fn detect_desktop() -> DesktopEnv {
    desktop_from_env(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAYLAND: Option<&str> = Some("wayland-1");

    #[test]
    fn no_wayland_display_is_always_other() {
        assert_eq!(
            desktop_from_env(None, Some("GNOME"), None),
            DesktopEnv::Other
        );
        assert_eq!(
            desktop_from_env(Some(""), Some("Hyprland"), Some("sig")),
            DesktopEnv::Other
        );
    }

    #[test]
    fn hyprland_signature_beats_xdg() {
        assert_eq!(
            desktop_from_env(WAYLAND, Some("GNOME"), Some("1758e0f0_1758_0")),
            DesktopEnv::Hyprland
        );
        // An EMPTY signature does not claim Hyprland.
        assert_eq!(
            desktop_from_env(WAYLAND, Some("sway"), Some("")),
            DesktopEnv::Sway
        );
    }

    #[test]
    fn xdg_token_table_covers_the_fallback_selection_desktops() {
        for (token, expected) in [
            ("Hyprland", DesktopEnv::Hyprland),
            ("sway", DesktopEnv::Sway),
            ("niri", DesktopEnv::Niri),
            ("COSMIC", DesktopEnv::Cosmic),
            ("kde", DesktopEnv::Kde),
            ("gnome", DesktopEnv::Gnome),
            ("GNOME-Wayland", DesktopEnv::Gnome),
            ("gnome-xorg", DesktopEnv::Gnome),
            ("gnome-classic", DesktopEnv::Gnome),
            ("gnome-flashback", DesktopEnv::Gnome),
            ("ubuntu:GNOME", DesktopEnv::Gnome),
            ("somethingelse", DesktopEnv::Other),
        ] {
            assert_eq!(
                desktop_from_env(WAYLAND, Some(token), None),
                expected,
                "token {token}"
            );
        }
        assert_eq!(desktop_from_env(WAYLAND, None, None), DesktopEnv::Other);
    }

    #[test]
    fn portal_override_table() {
        for value in ["0", "off", "FALSE", " disabled ", "Disabled"] {
            assert_eq!(
                PortalMode::from_override(Some(value)),
                PortalMode::Disabled,
                "value {value}"
            );
        }
        for value in [Some("1"), Some("on"), Some("maybe"), Some(""), None] {
            assert_eq!(PortalMode::from_override(value), PortalMode::Auto);
        }
        assert_eq!(PortalMode::default(), PortalMode::Auto);
    }
}
