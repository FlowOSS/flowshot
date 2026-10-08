//! The shortcut vocabulary: what a binding IS (id, description, trigger,
//! CLI invocation, daemon command) and the proposed defaults
//! (Print -> region, Shift+Print -> full, Ctrl+Print -> active monitor;
//! rebindable through the settings tab).

use std::collections::HashMap;

use crate::command::DaemonCommand;
use crate::request::CaptureRequest;

/// Sentinel screen index meaning "the output under the cursor" (the
/// "active monitor"). The cursor-to-output resolution lives in
/// the capture executor, which interprets this value; the daemon
/// only dispatches it. Mirrors the CLI's argument-less `flowshot capture
/// screen` (NO ARG = output under cursor).
pub const ACTIVE_SCREEN: u32 = u32::MAX;

/// One configurable global shortcut.
///
/// The same spec feeds BOTH registration paths: the portal path binds
/// [`Self::trigger`] (XDG "shortcuts" spec syntax, e.g. `Ctrl+Print`) and
/// dispatches [`Self::command`] on the portal's `Activated` signal; the
/// compositor-fallback path emits [`Self::cli`] into the paste-ready
/// snippet ([`super::fallback`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    /// Stable application-provided identifier (the portal's `shortcut_id`,
    /// the `Activated` signal key).
    pub id: String,
    /// User-readable purpose (the portal's `description`; the
    /// settings row label).
    pub description: String,
    /// Preferred trigger in XDG shortcut syntax (`Print`, `Shift+Print`,
    /// `Ctrl+Print`); the portal may assign a different one on conflict.
    pub trigger: String,
    /// CLI invocation for compositor-bind snippets.
    pub cli: String,
    /// Daemon command dispatched when the portal reports this id active.
    pub command: DaemonCommand,
}

/// The proposed defaults (Flameshot shortcut rows): Print -> region
/// capture, Shift+Print -> full, Ctrl+Print -> active monitor.
#[must_use]
pub fn default_shortcuts() -> Vec<ShortcutSpec> {
    vec![
        ShortcutSpec {
            id: "capture-region".to_owned(),
            description: "Capture a region".to_owned(),
            trigger: "Print".to_owned(),
            cli: "flowshot capture".to_owned(),
            command: DaemonCommand::Capture(CaptureRequest::default()),
        },
        ShortcutSpec {
            id: "capture-full".to_owned(),
            description: "Capture the full screen".to_owned(),
            trigger: "Shift+Print".to_owned(),
            cli: "flowshot capture --full".to_owned(),
            command: DaemonCommand::CaptureFull,
        },
        ShortcutSpec {
            id: "capture-active-monitor".to_owned(),
            description: "Capture the active monitor".to_owned(),
            trigger: "Ctrl+Print".to_owned(),
            cli: "flowshot capture screen".to_owned(),
            command: DaemonCommand::CaptureScreen(ACTIVE_SCREEN),
        },
    ]
}

/// The `Activated`-dispatch map: shortcut id -> command. Ids outside this
/// map are ignored (the portal broadcasts `Activated` for EVERY session on
/// the interface, so other applications' shortcuts arrive too).
#[must_use]
pub fn command_map(specs: &[ShortcutSpec]) -> HashMap<String, DaemonCommand> {
    specs
        .iter()
        .map(|spec| (spec.id.clone(), spec.command.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_f12_shortcut_rows() {
        let specs = default_shortcuts();
        let summary: Vec<(&str, &str, &str)> = specs
            .iter()
            .map(|spec| (spec.id.as_str(), spec.trigger.as_str(), spec.cli.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("capture-region", "Print", "flowshot capture"),
                ("capture-full", "Shift+Print", "flowshot capture --full"),
                (
                    "capture-active-monitor",
                    "Ctrl+Print",
                    "flowshot capture screen"
                ),
            ]
        );
    }

    #[test]
    fn defaults_map_to_the_daemon_command_vocabulary() {
        let map = command_map(&default_shortcuts());
        assert_eq!(
            map.get("capture-region"),
            Some(&DaemonCommand::Capture(CaptureRequest::default()))
        );
        assert_eq!(map.get("capture-full"), Some(&DaemonCommand::CaptureFull));
        assert_eq!(
            map.get("capture-active-monitor"),
            Some(&DaemonCommand::CaptureScreen(ACTIVE_SCREEN))
        );
        assert_eq!(map.get("not-ours"), None);
    }

    #[test]
    fn command_map_keys_every_spec_exactly_once() {
        let specs = default_shortcuts();
        let map = command_map(&specs);
        assert_eq!(map.len(), specs.len());
        for spec in &specs {
            assert_eq!(map.get(&spec.id), Some(&spec.command));
        }
    }
}
