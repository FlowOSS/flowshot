//! The compositor-bind fallback (plan todo-34 ladder rung 2, F14): when the
//! `GlobalShortcuts` portal is absent or denied (bare wlroots/sway per
//! xdp-wlr#240, or a masked portal), `FlowShot` generates PASTE-READY
//! compositor snippets instead of grabbing keys itself. Snippet binds invoke
//! the `flowshot` CLI directly and need NO resident daemon (Oracle r4 F-3 -
//! the residency difference vs the portal path is documented in the todo-40
//! per-desktop guides).
//!
//! [`bind_help`] is the single source for every surface the plan names:
//! the settings tab (todo 36 reads it), the docs (todo 40), and
//! `flowshot --print-bind-help` (todo 35). Golden-file tests pin the exact
//! output per flavor.
//!
//! KDE note (task brief "`KGlobalAccel` D-Bus"): KDE is served by the portal
//! in practice (xdp-kde, F14 ✅), so the plan's fallback ladder stays at
//! DOCUMENTED guidance for `KGlobalAccel` (System Settings / the
//! `org.kde.kglobalaccel` service) - no paste-ready file snippet exists for
//! Plasma's shortcut store. Recorded in the notepad.

use flowshot_capture::DesktopEnv;

use super::spec::ShortcutSpec;

/// Which snippet dialect a desktop's fallback emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositorFlavor {
    /// `bind = MODS,KEY,exec,CMD` lines for `hyprland.conf`.
    Hyprland,
    /// `bindsym TRIGGER exec CMD` lines for the sway config.
    Sway,
    /// `gsettings` custom-keybinding commands for GNOME.
    Gnome,
    /// Documented `KGlobalAccel` guidance for KDE Plasma.
    Kde,
    /// Command table + docs pointer for anything else (niri, COSMIC,
    /// unknown wlroots).
    #[default]
    Generic,
}

impl CompositorFlavor {
    /// The fallback-selection table (task brief: per-desktop, table-tested).
    #[must_use]
    pub const fn for_desktop(desktop: DesktopEnv) -> Self {
        match desktop {
            DesktopEnv::Hyprland => Self::Hyprland,
            DesktopEnv::Sway => Self::Sway,
            DesktopEnv::Gnome => Self::Gnome,
            DesktopEnv::Kde => Self::Kde,
            DesktopEnv::Niri | DesktopEnv::Cosmic | DesktopEnv::Other => Self::Generic,
        }
    }

    /// Stable log token.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Hyprland => "hyprland",
            Self::Sway => "sway",
            Self::Gnome => "gnome",
            Self::Kde => "kde",
            Self::Generic => "generic",
        }
    }

    /// The docs page name referenced by the help footer.
    #[must_use]
    pub const fn docs_page(&self) -> &'static str {
        match self {
            Self::Hyprland => "docs/setup-hyprland.md",
            Self::Sway => "docs/setup-sway.md",
            Self::Gnome => "docs/setup-gnome.md",
            Self::Kde => "docs/setup-kde.md",
            Self::Generic => "docs/",
        }
    }
}

/// The full `--print-bind-help` text for one flavor (deterministic; the
/// golden files under `tests/golden/` pin it byte-for-byte).
#[must_use]
pub fn bind_help(flavor: CompositorFlavor, specs: &[ShortcutSpec]) -> String {
    let mut out = String::from(HELP_HEADER);
    out.push_str("\n\n");
    match flavor {
        CompositorFlavor::Hyprland => {
            out.push_str(
                "# Hyprland - append to ~/.config/hypr/hyprland.conf, then `hyprctl reload`\n",
            );
            for spec in specs {
                out.push_str(&hyprland_bind(spec));
                out.push('\n');
            }
        }
        CompositorFlavor::Sway => {
            out.push_str("# Sway - append to ~/.config/sway/config, then `swaymsg reload`\n");
            for spec in specs {
                out.push_str(&sway_bind(spec));
                out.push('\n');
            }
        }
        CompositorFlavor::Gnome => {
            out.push_str(GNOME_PREAMBLE);
            out.push('\n');
            for line in gnome_keybinding_lines(specs) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        CompositorFlavor::Kde => {
            out.push_str(KDE_PREAMBLE);
            out.push('\n');
            for line in action_table(specs) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        CompositorFlavor::Generic => {
            out.push_str(GENERIC_PREAMBLE);
            out.push('\n');
            for line in action_table(specs) {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    out.push_str("Full per-desktop guide: ");
    out.push_str(flavor.docs_page());
    out.push('\n');
    out
}

const HELP_HEADER: &str = "\
FlowShot global shortcuts - compositor bind help

The org.freedesktop.portal.GlobalShortcuts portal is unavailable or was
denied, so FlowShot cannot grab hotkeys itself. Add the binds below to
your compositor configuration; they invoke the flowshot CLI directly and
need NO resident daemon.";

const GNOME_PREAMBLE: &str = "\
# GNOME - custom keybindings via gsettings (equivalently: System Settings
# -> Keyboard -> Keyboard Shortcuts -> Custom Shortcuts).
# WARNING: the first command REPLACES the custom-keybindings list; merge it
# with the output of
#   gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings
# when you already have custom bindings.";

const KDE_PREAMBLE: &str = "\
# KDE Plasma - KGlobalAccel
KDE normally serves the GlobalShortcuts portal through
xdg-desktop-portal-kde; reaching this fallback means the portal is missing
or was denied. Plasma has no paste-ready config snippet - register one
custom shortcut per action in System Settings -> Keyboard -> Shortcuts
(stored via KGlobalAccel, the org.kde.kglobalaccel D-Bus service):";

const GENERIC_PREAMBLE: &str = "\
# Your compositor
No paste-ready snippet is generated for this desktop. Bind these commands
in your compositor's keybind configuration (the triggers are suggestions;
any free key works). wlroots compositors generally accept the sway
`bindsym` syntax:";

/// One `hyprland.conf` bind line: `bind = MODS,KEY,exec,CMD`.
#[must_use]
pub fn hyprland_bind(spec: &ShortcutSpec) -> String {
    let (mods, key) = hyprland_trigger(&spec.trigger);
    format!("bind = {mods},{key},exec,{}", spec.cli)
}

/// One sway `bindsym` line (sway accepts the XDG trigger syntax verbatim).
#[must_use]
pub fn sway_bind(spec: &ShortcutSpec) -> String {
    format!("bindsym {} exec {}", spec.trigger, spec.cli)
}

/// Splits an XDG trigger into Hyprland's (MODS,KEY) bind fields: mods are
/// uppercase and `+`-joined (`CTRL+SHIFT`), the key passes through.
#[must_use]
pub fn hyprland_trigger(trigger: &str) -> (String, String) {
    let mut parts = trigger.split('+');
    let key = parts.next_back().unwrap_or_default().trim().to_owned();
    let mods: Vec<String> = parts
        .map(|token| match token.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "CTRL".to_owned(),
            "shift" => "SHIFT".to_owned(),
            "alt" => "ALT".to_owned(),
            "super" | "cmd" => "SUPER".to_owned(),
            other => other.to_ascii_uppercase(),
        })
        .collect();
    (mods.join("+"), key)
}

/// Converts an XDG trigger to the GTK accelerator syntax GNOME's
/// custom-keybinding `binding` key expects (`Ctrl+Print` -> `<Ctrl>Print`).
#[must_use]
pub fn gtk_accelerator(trigger: &str) -> String {
    let mut parts = trigger.split('+');
    let key = parts.next_back().unwrap_or_default().trim();
    let mut accel = String::new();
    for token in parts {
        let trimmed = token.trim();
        let modifier = match trimmed.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl",
            "shift" => "Shift",
            "alt" => "Alt",
            "super" | "cmd" => "Super",
            _ => trimmed,
        };
        accel.push('<');
        accel.push_str(modifier);
        accel.push('>');
    }
    accel.push_str(key);
    accel
}

/// The `gsettings` command block registering every spec as a GNOME custom
/// keybinding (paths are the conventional custom-keybindings namespace,
/// suffixed `flowshot<N>`).
#[must_use]
pub fn gnome_keybinding_lines(specs: &[ShortcutSpec]) -> Vec<String> {
    const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/flowshot";
    let paths: Vec<String> = (0..specs.len())
        .map(|index| format!("{BASE}{index}/"))
        .collect();
    let list = paths
        .iter()
        .map(|path| format!("'{path}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "gsettings set {MEDIA_KEYS} custom-keybindings \"[{list}]\""
    )];
    for (index, spec) in specs.iter().enumerate() {
        let path = &paths[index];
        lines.push(format!(
            "gsettings set {MEDIA_KEYS}.custom-keybinding:{path} name 'FlowShot: {}'",
            spec.description
        ));
        lines.push(format!(
            "gsettings set {MEDIA_KEYS}.custom-keybinding:{path} command '{}'",
            spec.cli
        ));
        lines.push(format!(
            "gsettings set {MEDIA_KEYS}.custom-keybinding:{path} binding '{}'",
            gtk_accelerator(&spec.trigger)
        ));
    }
    lines
}

/// The aligned `trigger -> cli (description)` table used by the guidance
/// flavors (KDE/Generic).
#[must_use]
pub fn action_table(specs: &[ShortcutSpec]) -> Vec<String> {
    let trigger_width = specs
        .iter()
        .map(|spec| spec.trigger.chars().count())
        .max()
        .unwrap_or_default();
    let cli_width = specs
        .iter()
        .map(|spec| spec.cli.chars().count())
        .max()
        .unwrap_or_default();
    specs
        .iter()
        .map(|spec| {
            format!(
                "  {:<trigger_width$}  ->  {:<cli_width$}  ({})",
                spec.trigger, spec.cli, spec.description
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcut::spec::default_shortcuts;

    #[test]
    fn fallback_selection_table_per_desktop() {
        for (desktop, expected) in [
            (DesktopEnv::Hyprland, CompositorFlavor::Hyprland),
            (DesktopEnv::Sway, CompositorFlavor::Sway),
            (DesktopEnv::Kde, CompositorFlavor::Kde),
            (DesktopEnv::Gnome, CompositorFlavor::Gnome),
            (DesktopEnv::Niri, CompositorFlavor::Generic),
            (DesktopEnv::Cosmic, CompositorFlavor::Generic),
            (DesktopEnv::Other, CompositorFlavor::Generic),
        ] {
            assert_eq!(
                CompositorFlavor::for_desktop(desktop),
                expected,
                "desktop {desktop:?}"
            );
        }
    }

    #[test]
    fn hyprland_trigger_splits_mods_and_key() {
        assert_eq!(
            hyprland_trigger("Print"),
            (String::new(), "Print".to_owned())
        );
        assert_eq!(
            hyprland_trigger("Shift+Print"),
            ("SHIFT".to_owned(), "Print".to_owned())
        );
        assert_eq!(
            hyprland_trigger("Ctrl+Print"),
            ("CTRL".to_owned(), "Print".to_owned())
        );
        assert_eq!(
            hyprland_trigger("Ctrl+Shift+F13"),
            ("CTRL+SHIFT".to_owned(), "F13".to_owned())
        );
        assert_eq!(
            hyprland_trigger("Super+p"),
            ("SUPER".to_owned(), "p".to_owned())
        );
    }

    #[test]
    fn gtk_accelerator_wraps_modifiers() {
        assert_eq!(gtk_accelerator("Print"), "Print");
        assert_eq!(gtk_accelerator("Shift+Print"), "<Shift>Print");
        assert_eq!(gtk_accelerator("Ctrl+Print"), "<Ctrl>Print");
        assert_eq!(gtk_accelerator("Ctrl+Alt+F1"), "<Ctrl><Alt>F1");
    }

    #[test]
    fn snippet_lines_use_the_plan_verbatim_forms() {
        let specs = default_shortcuts();
        // The plan's exact examples: hyprland `bind = ,Print,exec,flowshot
        // capture`; sway `bindsym Print exec flowshot capture`.
        assert_eq!(
            hyprland_bind(&specs[0]),
            "bind = ,Print,exec,flowshot capture"
        );
        assert_eq!(sway_bind(&specs[0]), "bindsym Print exec flowshot capture");
        assert_eq!(
            hyprland_bind(&specs[2]),
            "bind = CTRL,Print,exec,flowshot capture screen"
        );
        assert_eq!(
            sway_bind(&specs[1]),
            "bindsym Shift+Print exec flowshot capture --full"
        );
    }

    #[test]
    fn bind_help_matches_the_golden_files() {
        let specs = default_shortcuts();
        for (flavor, golden) in [
            (
                CompositorFlavor::Hyprland,
                include_str!("../../tests/golden/bind-help-hyprland.txt"),
            ),
            (
                CompositorFlavor::Sway,
                include_str!("../../tests/golden/bind-help-sway.txt"),
            ),
            (
                CompositorFlavor::Gnome,
                include_str!("../../tests/golden/bind-help-gnome.txt"),
            ),
            (
                CompositorFlavor::Kde,
                include_str!("../../tests/golden/bind-help-kde.txt"),
            ),
            (
                CompositorFlavor::Generic,
                include_str!("../../tests/golden/bind-help-generic.txt"),
            ),
        ] {
            assert_eq!(
                bind_help(flavor, &specs),
                golden,
                "golden mismatch for {}",
                flavor.label()
            );
        }
    }

    #[test]
    fn gnome_lines_cover_every_spec_with_three_keys_each() {
        let specs = default_shortcuts();
        let lines = gnome_keybinding_lines(&specs);
        // 1 list-set + 3 specs x (name, command, binding).
        assert_eq!(lines.len(), 1 + specs.len() * 3);
        assert!(lines[0].contains("custom-keybindings \"["));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("binding '<Ctrl>Print'"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("command 'flowshot capture --full'"))
        );
    }

    #[test]
    fn action_table_is_aligned_and_complete() {
        let table = action_table(&default_shortcuts());
        assert_eq!(table.len(), 3);
        for line in &table {
            assert!(line.contains("->"), "line: {line}");
            assert!(line.starts_with("  "), "line: {line}");
        }
        assert!(table[0].contains("Print"));
        assert!(table[2].contains("(Capture the active monitor)"));
    }

    /// Regenerates the golden files after an INTENTIONAL help-text change:
    /// `cargo test -p flowshot-daemon --lib regenerate_goldens -- --ignored`
    /// then review the diff.
    #[test]
    #[ignore = "golden generator; run explicitly after intentional help-text changes"]
    fn regenerate_goldens() {
        let specs = default_shortcuts();
        for (flavor, name) in [
            (CompositorFlavor::Hyprland, "hyprland"),
            (CompositorFlavor::Sway, "sway"),
            (CompositorFlavor::Gnome, "gnome"),
            (CompositorFlavor::Kde, "kde"),
            (CompositorFlavor::Generic, "generic"),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("golden")
                .join(format!("bind-help-{name}.txt"));
            std::fs::write(&path, bind_help(flavor, &specs))
                .unwrap_or_else(|error| panic!("{path:?}: {error}"));
        }
    }
}
