//! Resilient config loading for CLI-side validation.
//!
//! Same rule as the daemon binary (todo-2 resilience): a missing,
//! unreadable, or corrupt config file NEVER fails the command - defaults
//! apply and a warning is logged. The CLI reads config only where the plan
//! mandates a CLI-side gate (the `--upload` client-id check, Amendment #3).

use std::path::Path;

use flowshot_core::Config;

/// Loads the config from `explicit` or the default XDG path, falling back
/// to defaults on any problem.
pub fn load(explicit: Option<&Path>) -> Config {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => match flowshot_daemon::paths::default_config_path() {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "could not resolve the config path; using defaults");
                return Config::default();
            }
        },
    };
    match Config::load(&path) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "config load failed; using defaults");
            Config::default()
        }
    }
}
