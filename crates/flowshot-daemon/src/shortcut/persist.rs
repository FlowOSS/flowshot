//! Restore-data persistence ("persist portal restore-data in
//! config dir"; acceptance: "restore-data file created + reused across
//! daemon restarts").
//!
//! PINNED API REALITY (ashpd 0.13.13, source-verified - the
//! "restore-data" wording assumes `ScreenCast`-style restore tokens):
//! `GlobalShortcuts` has NO `restore_token`/`persist_mode`, and ashpd
//! exposes neither the session handle nor the handle tokens
//! (`Session` has no public path accessor and `HandleToken` is
//! `pub(crate)` without the `backend` feature). A portal session
//! therefore CANNOT survive a daemon restart; what persists is the
//! RE-REGISTRATION set (ids, descriptions, triggers, the portal-assigned
//! trigger descriptions) plus the one-time-notification state, so a restart
//! re-binds the identical shortcuts without re-nagging the user. The
//! settings tab reads this file to list the registered shortcuts.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::spec::ShortcutSpec;
use crate::error::DaemonError;

/// Current on-disk schema version (bump + migrate when the shape changes).
pub const RESTORE_DATA_VERSION: u32 = 1;

/// One registered shortcut as persisted (the settings row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedShortcut {
    /// The application-provided id.
    pub id: String,
    /// User-readable purpose.
    pub description: String,
    /// The trigger `FlowShot` asked for (XDG shortcut syntax).
    pub preferred_trigger: String,
    /// The trigger the portal actually assigned (may differ on conflict).
    pub trigger_description: String,
}

/// The persisted registration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreData {
    /// Schema version.
    pub version: u32,
    /// Whether the one-time autostart-recommendation notification was
    /// already emitted (the FIRST-successful-registration rule).
    pub first_registration_notified: bool,
    /// When the current registration was recorded.
    pub registered_at: Option<SystemTime>,
    /// The registered shortcuts.
    pub shortcuts: Vec<PersistedShortcut>,
}

impl Default for RestoreData {
    fn default() -> Self {
        Self {
            version: RESTORE_DATA_VERSION,
            first_registration_notified: false,
            registered_at: None,
            shortcuts: Vec::new(),
        }
    }
}

impl RestoreData {
    /// Builds the record from the requested specs and the portal's bound
    /// reply (id, description, portal-assigned trigger description).
    /// Entries the portal dropped are absent from `bound` and keep their
    /// requested trigger as the description (honest: nothing was assigned).
    #[must_use]
    pub fn from_bound(
        specs: &[ShortcutSpec],
        bound: &[(String, String, String)],
        notified: bool,
        now: SystemTime,
    ) -> Self {
        let shortcuts = specs
            .iter()
            .map(|spec| {
                let assigned = bound
                    .iter()
                    .find(|(id, _, _)| *id == spec.id)
                    .map_or_else(|| spec.trigger.clone(), |(_, _, trigger)| trigger.clone());
                PersistedShortcut {
                    id: spec.id.clone(),
                    description: spec.description.clone(),
                    preferred_trigger: spec.trigger.clone(),
                    trigger_description: assigned,
                }
            })
            .collect();
        Self {
            version: RESTORE_DATA_VERSION,
            first_registration_notified: notified,
            registered_at: Some(now),
            shortcuts,
        }
    }

    /// Loads the file: absent -> `None` (first run); corrupt -> `None` with
    /// a warning (resilience rule: never fail the daemon over a
    /// data file - the cost is one repeated autostart nudge, documented).
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(data) => Some(data),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "shortcut restore data is corrupt; treating as first run");
                    None
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "shortcut restore data is unreadable; treating as first run");
                None
            }
        }
    }

    /// Writes the file (creating the config dir).
    ///
    /// # Errors
    ///
    /// [`DaemonError::Io`] when the directory or file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), DaemonError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcut::spec::default_shortcuts;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flowshot-restore-{}-{tag}.json",
            std::process::id()
        ))
    }

    fn bound() -> Vec<(String, String, String)> {
        vec![
            (
                "capture-region".to_owned(),
                "Capture a region".to_owned(),
                "Print".to_owned(),
            ),
            (
                "capture-full".to_owned(),
                "Capture the full screen".to_owned(),
                "Shift+Print".to_owned(),
            ),
        ]
    }

    #[test]
    fn save_load_roundtrip_is_lossless() {
        let path = temp_path("roundtrip");
        let data =
            RestoreData::from_bound(&default_shortcuts(), &bound(), true, SystemTime::UNIX_EPOCH);
        data.save(&path).unwrap_or_else(|error| panic!("{error}"));
        let loaded = RestoreData::load(&path);
        assert_eq!(loaded, Some(data));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_is_none_and_corrupt_file_is_none() {
        let path = temp_path("missing");
        std::fs::remove_file(&path).ok();
        assert_eq!(RestoreData::load(&path), None);

        std::fs::write(&path, b"{not json").ok();
        assert_eq!(RestoreData::load(&path), None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_bound_pairs_assigned_triggers_and_keeps_dropped_specs() {
        let specs = default_shortcuts();
        let data = RestoreData::from_bound(&specs, &bound(), false, SystemTime::UNIX_EPOCH);
        assert_eq!(data.version, RESTORE_DATA_VERSION);
        assert!(!data.first_registration_notified);
        assert_eq!(data.shortcuts.len(), specs.len());
        assert_eq!(data.shortcuts[0].trigger_description, "Print");
        assert_eq!(data.shortcuts[1].trigger_description, "Shift+Print");
        // The portal dropped the third shortcut: its persisted trigger
        // description falls back to the requested trigger.
        assert_eq!(data.shortcuts[2].id, "capture-active-monitor");
        assert_eq!(data.shortcuts[2].trigger_description, "Ctrl+Print");
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let base =
            std::env::temp_dir().join(format!("flowshot-restore-dir-{}", std::process::id()));
        let path = base.join("nested").join("shortcuts-restore.json");
        std::fs::remove_dir_all(&base).ok();
        let data = RestoreData::default();
        data.save(&path).unwrap_or_else(|error| panic!("{error}"));
        assert!(path.exists());
        std::fs::remove_dir_all(&base).ok();
    }
}
