//! XDG path resolution for the daemon: the config file location and the
//! autostart directory. Environment reading is confined to
//! [`xdg_config_home`]; every consumer takes an injected base so tests
//! never mutate process env (parallel-test race avoidance).

use std::path::{Path, PathBuf};

use crate::error::DaemonError;

/// The product config directory name under the XDG config home.
pub const CONFIG_DIR_NAME: &str = "flowshot";

/// The config file name (`~/.config/flowshot/flowshot.toml`).
pub const CONFIG_FILE_NAME: &str = "flowshot.toml";

/// The shortcut restore-data file name ("persist portal
/// restore-data in config dir"; contents per [`crate::shortcut::persist`]).
pub const SHORTCUTS_RESTORE_FILE_NAME: &str = "shortcuts-restore.json";

/// Resolves `$XDG_CONFIG_HOME`, falling back to `$HOME/.config` (the XDG
/// Base Directory spec default).
///
/// # Errors
///
/// [`DaemonError::Env`] when neither variable yields a usable path.
pub fn xdg_config_home() -> Result<PathBuf, DaemonError> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or(DaemonError::Env("HOME"))?;
    if home.is_empty() {
        return Err(DaemonError::Env("HOME"));
    }
    Ok(PathBuf::from(home).join(".config"))
}

/// The default config file path: `<xdg_config_home>/flowshot/flowshot.toml`.
///
/// # Errors
///
/// [`DaemonError::Env`] when the config home cannot be resolved.
pub fn default_config_path() -> Result<PathBuf, DaemonError> {
    Ok(config_path_in(&xdg_config_home()?))
}

/// Pure path composition (the injected-base twin of
/// [`default_config_path`]).
#[must_use]
pub fn config_path_in(config_home: &Path) -> PathBuf {
    config_home.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME)
}

/// The default shortcut restore-data path:
/// `<xdg_config_home>/flowshot/shortcuts-restore.json`.
///
/// # Errors
///
/// [`DaemonError::Env`] when the config home cannot be resolved.
pub fn default_shortcuts_restore_path() -> Result<PathBuf, DaemonError> {
    Ok(shortcuts_restore_path_in(&xdg_config_home()?))
}

/// Pure path composition (the injected-base twin of
/// [`default_shortcuts_restore_path`]).
#[must_use]
pub fn shortcuts_restore_path_in(config_home: &Path) -> PathBuf {
    config_home
        .join(CONFIG_DIR_NAME)
        .join(SHORTCUTS_RESTORE_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_composes_under_the_given_home() {
        let path = config_path_in(Path::new("/home/tester/.config"));
        assert_eq!(
            path,
            PathBuf::from("/home/tester/.config/flowshot/flowshot.toml")
        );
    }

    #[test]
    fn xdg_config_home_resolves_or_errors_without_panic() {
        match xdg_config_home() {
            Ok(dir) => assert!(dir.is_absolute() || dir.starts_with(".")),
            Err(error) => assert!(matches!(error, DaemonError::Env(_))),
        }
    }
}
