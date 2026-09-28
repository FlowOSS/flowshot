//! Post-capture action-set semantics.
//!
//! The ordered `[save].actions` set `{copy, copy-path, save, pin, upload,
//! notify, open-with}` replaces Flameshot's `saveAfterCopy` /
//! `copyPathAfterSave` boolean flags. This module owns the two sequencing
//! rules:
//!
//! - MERGE RULE: effective set = configured order
//!   first, then legacy-flag-implied additions deduped at the end.
//! - `copy-path` executes AFTER the last `save` in the effective
//!   sequence; with no save in the sequence the order is unchanged and
//!   the runtime executor warns + no-ops.

use flowshot_core::config::SaveAction;
use serde::{Deserialize, Serialize};

/// One post-capture action from the ordered `[save].actions` set.
///
/// Serializes with the kebab-case vocabulary (`copy-path`,
/// `open-with`). The core config enum ([`SaveAction`]) currently covers
/// only the four variants it can round-trip; the CLI and daemon
/// construct the remaining variants per invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Copy the image to the clipboard.
    Copy,
    /// Copy the saved file's path (`text/plain` + `text/uri-list`).
    CopyPath,
    /// Write the image to disk.
    Save,
    /// Open the image as a floating pin (see [`crate::pin`]).
    Pin,
    /// Upload the image (see [`crate::upload`]).
    Upload,
    /// Explicit success-toast request (gated by `[daemon].notifications`).
    Notify,
    /// Open the saved file with the default application (portal).
    OpenWith,
}

impl From<SaveAction> for Action {
    fn from(action: SaveAction) -> Self {
        match action {
            SaveAction::Copy => Self::Copy,
            SaveAction::Save => Self::Save,
            SaveAction::Pin => Self::Pin,
            SaveAction::Upload => Self::Upload,
            SaveAction::CopyPath => Self::CopyPath,
            SaveAction::Notify => Self::Notify,
            SaveAction::OpenWith => Self::OpenWith,
        }
    }
}

/// Legacy Flameshot boolean action flags, for config migration.
///
/// The ordered `[save].actions` set replaced these;
/// the merge rule below folds configs that still carry them into an
/// effective action sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyFlags {
    /// Flameshot `saveAfterCopy`: implies `copy` + `save`.
    pub save_after_copy: bool,
    /// Flameshot `copyPathAfterSave`: implies `save` + `copy-path`.
    pub copy_path_after_save: bool,
}

/// MERGE RULE: effective set = configured order first,
/// then flag-implied additions deduped at the end.
#[must_use]
pub fn effective_actions(configured: &[Action], flags: LegacyFlags) -> Vec<Action> {
    let mut effective = configured.to_vec();
    let mut implied: Vec<Action> = Vec::new();
    if flags.save_after_copy {
        implied.extend([Action::Copy, Action::Save]);
    }
    if flags.copy_path_after_save {
        implied.extend([Action::Save, Action::CopyPath]);
    }
    for action in implied {
        if !effective.contains(&action) {
            effective.push(action);
        }
    }
    effective
}

/// Execution order for an effective action set: every `copy-path` runs
/// AFTER the last `save`. Only VIOLATING entries (a `copy-path`
/// positioned before the last `save`) move — to just after that save —
/// so the configured order is otherwise preserved. With no `save` in
/// the sequence the order is unchanged; the executor warns + no-ops
/// `copy-path` at runtime ("warn+no-op when no save occurred").
#[must_use]
pub fn execution_order(effective: &[Action]) -> Vec<Action> {
    let Some(last_save) = effective.iter().rposition(|action| *action == Action::Save) else {
        return effective.to_vec();
    };
    let mut ordered: Vec<Action> = Vec::with_capacity(effective.len());
    let mut moved: Vec<Action> = Vec::new();
    for (index, action) in effective.iter().copied().enumerate() {
        if action == Action::CopyPath && index < last_save {
            moved.push(action);
        } else {
            ordered.push(action);
        }
    }
    if moved.is_empty() {
        return effective.to_vec();
    }
    // The last save kept its relative position minus the entries pulled
    // out before it; the moved copy-paths land directly behind it.
    let insert_at = last_save - moved.len() + 1;
    ordered.splice(insert_at..insert_at, moved);
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::{Copy, CopyPath, Notify, OpenWith, Pin, Save, Upload};

    fn flags(save_after_copy: bool, copy_path_after_save: bool) -> LegacyFlags {
        LegacyFlags {
            save_after_copy,
            copy_path_after_save,
        }
    }

    #[test]
    fn merge_rule_table() {
        let cases: Vec<(&[Action], LegacyFlags, &[Action])> = vec![
            // No flags: configured order preserved verbatim.
            (&[Copy, Save], flags(false, false), &[Copy, Save]),
            (&[Save, Copy], flags(false, false), &[Save, Copy]),
            // saveAfterCopy implies copy+save, deduped at the end.
            (&[Copy], flags(true, false), &[Copy, Save]),
            (&[Pin], flags(true, false), &[Pin, Copy, Save]),
            // copyPathAfterSave implies save+copy-path.
            (&[Copy, Save], flags(false, true), &[Copy, Save, CopyPath]),
            (&[Notify], flags(false, true), &[Notify, Save, CopyPath]),
            // Both flags: additions deduped against each other too.
            (&[], flags(true, true), &[Copy, Save, CopyPath]),
            (
                &[OpenWith],
                flags(true, true),
                &[OpenWith, Copy, Save, CopyPath],
            ),
            // Everything already configured: nothing added.
            (
                &[Copy, Save, CopyPath],
                flags(true, true),
                &[Copy, Save, CopyPath],
            ),
            // Upload survives untouched.
            (&[Upload], flags(true, false), &[Upload, Copy, Save]),
        ];
        for (configured, legacy, expected) in cases {
            assert_eq!(effective_actions(configured, legacy), expected);
        }
    }

    #[test]
    fn execution_order_moves_copy_path_after_last_save() {
        let cases: Vec<(&[Action], &[Action])> = vec![
            (&[CopyPath, Save], &[Save, CopyPath]),
            (&[CopyPath, Save, Pin, Save], &[Save, Pin, Save, CopyPath]),
            (
                &[Copy, CopyPath, Save, CopyPath],
                &[Copy, Save, CopyPath, CopyPath],
            ),
            // Minimal relocation: only the violating entry moves, a
            // compliant one keeps its configured position.
            (
                &[CopyPath, Save, Pin, CopyPath],
                &[Save, CopyPath, Pin, CopyPath],
            ),
            // Already after the last save: unchanged.
            (&[Save, Copy, CopyPath], &[Save, Copy, CopyPath]),
            // No save at all: unchanged (runtime warns + no-ops).
            (&[CopyPath, Copy], &[CopyPath, Copy]),
            (&[CopyPath], &[CopyPath]),
            // No copy-path: unchanged.
            (&[Save, Copy, Pin], &[Save, Copy, Pin]),
            (&[], &[]),
        ];
        for (effective, expected) in cases {
            assert_eq!(execution_order(effective), expected);
        }
    }

    #[test]
    fn core_save_action_maps_into_action() {
        assert_eq!(Action::from(SaveAction::Copy), Copy);
        assert_eq!(Action::from(SaveAction::Save), Save);
        assert_eq!(Action::from(SaveAction::Pin), Pin);
        assert_eq!(Action::from(SaveAction::Upload), Upload);
    }

    #[test]
    fn serde_uses_kebab_case_vocabulary() {
        // Wire-format tokens the config/CLI parse — not prose.
        assert_eq!(serde_json::to_string(&Copy).unwrap_or_default(), "\"copy\"");
        assert_eq!(
            serde_json::to_string(&CopyPath).unwrap_or_default(),
            "\"copy-path\""
        );
        assert_eq!(
            serde_json::to_string(&OpenWith).unwrap_or_default(),
            "\"open-with\""
        );
        assert_eq!(
            serde_json::from_str::<Action>("\"notify\"").ok(),
            Some(Notify)
        );
        assert_eq!(
            serde_json::from_str::<Action>("\"open-with\"").ok(),
            Some(OpenWith)
        );
    }
}
