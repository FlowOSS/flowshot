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

/// The per-install telemetry identifier file name under the XDG data home
/// ([`crate::telemetry`] tier-2 payload; deleting the file regenerates it).
pub const TELEMETRY_ID_FILE_NAME: &str = "telemetry-id";

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
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA")
        && !appdata.is_empty()
    {
        return Ok(PathBuf::from(appdata));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or(DaemonError::Env("HOME or USERPROFILE"))?;
    if home.is_empty() {
        return Err(DaemonError::Env("HOME or USERPROFILE"));
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

/// Resolves `$XDG_DATA_HOME`, falling back to `$HOME/.local/share` (the XDG
/// Base Directory spec default).
///
/// # Errors
///
/// [`DaemonError::Env`] when neither variable yields a usable path.
pub fn xdg_data_home() -> Result<PathBuf, DaemonError> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(windows)]
    if let Some(localappdata) = std::env::var_os("LOCALAPPDATA")
        && !localappdata.is_empty()
    {
        return Ok(PathBuf::from(localappdata));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or(DaemonError::Env("HOME or USERPROFILE"))?;
    if home.is_empty() {
        return Err(DaemonError::Env("HOME or USERPROFILE"));
    }
    Ok(PathBuf::from(home).join(".local").join("share"))
}

/// The default telemetry install-id path:
/// `<xdg_data_home>/flowshot/telemetry-id`.
///
/// # Errors
///
/// [`DaemonError::Env`] when the data home cannot be resolved.
pub fn default_telemetry_id_path() -> Result<PathBuf, DaemonError> {
    Ok(telemetry_id_path_in(&xdg_data_home()?))
}

/// Pure path composition (the injected-base twin of
/// [`default_telemetry_id_path`]).
#[must_use]
pub fn telemetry_id_path_in(data_home: &Path) -> PathBuf {
    data_home.join(CONFIG_DIR_NAME).join(TELEMETRY_ID_FILE_NAME)
}

/// Pure logic to heal a malformed or incomplete `XDG_DATA_DIRS` string.
///
/// Ensures `/usr/local/share` and `/usr/share` exist in the path list, and
/// filters out unexpanded shell tokens like `$XDG_DATA_DIRS`.
#[must_use]
pub fn heal_xdg_data_dirs_str(raw: &str) -> Option<String> {
    const STANDARD_DIRS: [&str; 2] = ["/usr/local/share", "/usr/share"];

    let has_usr_share = raw
        .split(':')
        .any(|p| p == "/usr/share" || p == "/usr/share/");
    let has_broken_var = raw.contains("$XDG_DATA_DIRS") || raw.contains("${XDG_DATA_DIRS}");

    if has_usr_share && !has_broken_var && !raw.is_empty() {
        return None;
    }

    let mut dirs: Vec<String> = raw
        .split(':')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.contains('$'))
        .map(ToString::to_string)
        .collect();

    for std_dir in STANDARD_DIRS {
        if !dirs.iter().any(|d| d == std_dir) {
            dirs.push(std_dir.to_string());
        }
    }

    Some(dirs.join(":"))
}

/// Inspects and self-heals `$XDG_DATA_DIRS` if it is missing, malformed, or
/// lacks standard `/usr/share` system directories (e.g. from broken shell
/// exports like `XDG_DATA_DIRS=$XDG_DATA_DIRS:...` with unexpanded quotes).
///
/// Returns `Some((raw, healed))` if an issue was detected and self-healed,
/// or `None` if the environment was already healthy.
#[cfg(unix)]
#[must_use]
pub fn check_and_heal_xdg_data_dirs() -> Option<(String, String)> {
    let raw = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    heal_xdg_data_dirs_str(&raw).map(|healed| (raw, healed))
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

    #[test]
    fn xdg_data_dirs_heals_unexpanded_literal_tokens() {
        let broken = "$XDG_DATA_DIRS:/var/lib/flatpak/exports/share:/home/user/.local/share";
        let Some(healed) = heal_xdg_data_dirs_str(broken) else {
            panic!("expected healed string");
        };
        assert_eq!(
            healed,
            "/var/lib/flatpak/exports/share:/home/user/.local/share:/usr/local/share:/usr/share"
        );
    }

    #[test]
    fn xdg_data_dirs_heals_missing_usr_share() {
        let missing = "/var/lib/flatpak/exports/share";
        let Some(healed) = heal_xdg_data_dirs_str(missing) else {
            panic!("expected healed string");
        };
        assert_eq!(
            healed,
            "/var/lib/flatpak/exports/share:/usr/local/share:/usr/share"
        );
    }

    #[test]
    fn xdg_data_dirs_healthy_string_returns_none() {
        let healthy = "/usr/local/share:/usr/share:/var/lib/flatpak/exports/share";
        assert_eq!(heal_xdg_data_dirs_str(healthy), None);
    }
}
