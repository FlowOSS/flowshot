//! Session autostart: the `[daemon].startup_launch` config key mapped to
//! an XDG `.desktop` autostart entry.
//!
//! DECISION (recorded): the `auto-launch` crate is absent from the
//! workspace table AND `Cargo.lock`, and
//! the root manifest is orchestrator-owned - so the entry is written
//! directly (the crate's Linux backend does exactly this). Zero new
//! dependencies, injectable directory, fully unit-testable.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::paths::xdg_config_home;
use crate::strings;

/// The autostart entry file name (product-level, XDG-normal).
pub const ENTRY_FILE_NAME: &str = "flowshot.desktop";

/// Manages the autostart entry inside one directory (production:
/// `<xdg_config_home>/autostart`; tests inject a temp dir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autostart {
    dir: PathBuf,
}

impl Autostart {
    /// Resolves the production autostart directory from the environment.
    ///
    /// # Errors
    ///
    /// [`DaemonError::Env`] when the XDG config home cannot be resolved.
    pub fn from_env() -> Result<Self, DaemonError> {
        Ok(Self {
            dir: xdg_config_home()?.join("autostart"),
        })
    }

    /// An autostart manager writing into `dir` (test/QA seam).
    #[must_use]
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The entry file path.
    #[must_use]
    pub fn entry_path(&self) -> PathBuf {
        self.dir.join(ENTRY_FILE_NAME)
    }

    /// Whether the entry currently exists.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.entry_path().exists()
    }

    /// Writes (or rewrites) the entry launching `exec` at session start.
    ///
    /// # Errors
    ///
    /// [`DaemonError::Io`] when the directory or file cannot be written.
    pub fn enable(&self, exec: &str) -> Result<(), DaemonError> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.entry_path(), entry_contents(exec))?;
        tracing::info!(entry = %self.entry_path().display(), "autostart entry enabled");
        Ok(())
    }

    /// Removes the entry; a missing entry is success (idempotent).
    ///
    /// # Errors
    ///
    /// [`DaemonError::Io`] when the file exists but cannot be removed.
    pub fn disable(&self) -> Result<(), DaemonError> {
        match std::fs::remove_file(self.entry_path()) {
            Ok(()) => {
                tracing::info!(entry = %self.entry_path().display(), "autostart entry removed");
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(DaemonError::Io(error)),
        }
    }

    /// Applies the `[daemon].startup_launch` config value: `true` ensures
    /// the entry exists, `false` ensures it does not.
    ///
    /// # Errors
    ///
    /// Whatever [`Self::enable`] / [`Self::disable`] report.
    pub fn sync(&self, startup_launch: bool, exec: &str) -> Result<(), DaemonError> {
        if startup_launch {
            self.enable(exec)
        } else {
            self.disable()
        }
    }
}

/// The `.desktop` entry text for `exec` (pure; validated with
/// `desktop-file-validate` in live QA).
#[must_use]
pub fn entry_contents(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         GenericName={generic}\n\
         Comment={comment}\n\
         Exec={exec}\n\
         Terminal=false\n\
         Categories=Utility;\n\
         X-GNOME-Autostart-enabled=true\n",
        name = strings::AUTOSTART_NAME,
        generic = strings::AUTOSTART_GENERIC_NAME,
        comment = strings::AUTOSTART_COMMENT,
    )
}

/// Quotes an executable path for a `.desktop` `Exec=` value when it
/// contains spaces (the desktop-entry spec's quoting rule).
#[must_use]
pub fn exec_value(exe: &Path) -> String {
    let text = exe.to_string_lossy();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

/// The stable path daemon-related launches should reference: `$APPIMAGE`
/// when running inside an `AppImage`, else the current executable.
///
/// Inside an `AppImage` `current_exe` is the ephemeral FUSE mount
/// (`/tmp/.mount_*`) that disappears at unmount, while the runtime's
/// `$APPIMAGE` variable holds the stable file path (documented contract:
/// "shall be used every time the full path of the `AppImage` is needed").
/// Split from the env reads so tests never mutate the process env (the
/// [`crate::paths`] convention).
#[must_use]
pub fn launch_target_from(appimage: Option<OsString>, exe: Option<PathBuf>) -> Option<PathBuf> {
    appimage
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or(exe)
}

/// Live-env form of [`launch_target_from`].
#[must_use]
pub fn launch_target() -> Option<PathBuf> {
    launch_target_from(std::env::var_os("APPIMAGE"), std::env::current_exe().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn launch_target_prefers_the_stable_appimage_path() {
        // Inside an AppImage the current exe is the ephemeral FUSE mount;
        // $APPIMAGE is the stable file path and must win.
        let mounted = PathBuf::from("/tmp/.mount_FlowSh1a2b3c/usr/bin/flowshot");
        let appimage = PathBuf::from("/home/user/Apps/FlowShot-0.1.0-x86_64.AppImage");
        assert_eq!(
            launch_target_from(Some(appimage.clone().into_os_string()), Some(mounted)),
            Some(appimage)
        );
        // An empty $APPIMAGE is ignored: the current exe is the target.
        let exe = PathBuf::from("/usr/bin/flowshot");
        assert_eq!(
            launch_target_from(Some(OsString::new()), Some(exe.clone())),
            Some(exe.clone())
        );
        assert_eq!(launch_target_from(None, Some(exe.clone())), Some(exe));
        assert_eq!(launch_target_from(None, None), None);
    }

    #[test]
    fn cli_path_autostart_entries_launch_the_daemon_not_a_capture() {
        // Regression: a daemon started through the `flowshot` CLI must
        // write an Exec carrying the `daemon` subcommand - the bare CLI
        // invocation is an interactive capture, which would pop a
        // selection overlay at next login instead of starting the daemon.
        let exec = format!("{} daemon", exec_value(Path::new("/usr/bin/flowshot")));
        assert!(entry_contents(&exec).contains("Exec=/usr/bin/flowshot daemon\n"));
        // Paths with spaces keep the desktop-entry quoting rule.
        let quoted = format!("{} daemon", exec_value(Path::new("/opt/my apps/flowshot")));
        assert!(entry_contents(&quoted).contains("Exec=\"/opt/my apps/flowshot\" daemon\n"));
        // An AppImage target resolves to the stable path + subcommand.
        let target = launch_target_from(
            Some(OsString::from("/home/user/FlowShot.AppImage")),
            Some(PathBuf::from("/tmp/.mount_x/usr/bin/flowshot")),
        )
        .unwrap_or_else(|| panic!("launch target must resolve"));
        let exec = format!("{} daemon", exec_value(&target));
        assert!(entry_contents(&exec).contains("Exec=/home/user/FlowShot.AppImage daemon\n"));
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "flowshot-autostart-test-{}-{tag}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn enable_writes_a_valid_entry_and_disable_removes_it() {
        let dir = temp_dir("roundtrip");
        let autostart = Autostart::with_dir(&dir);
        assert!(!autostart.is_enabled());

        autostart
            .enable("/usr/bin/flowshot-daemon")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(autostart.is_enabled());
        let contents = std::fs::read_to_string(autostart.entry_path())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(contents.starts_with("[Desktop Entry]\n"));
        assert!(contents.contains("Type=Application\n"));
        assert!(contents.contains("Exec=/usr/bin/flowshot-daemon\n"));
        assert!(contents.contains("X-GNOME-Autostart-enabled=true\n"));

        autostart
            .disable()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!autostart.is_enabled());
        // Idempotent: disabling twice is not an error.
        autostart
            .disable()
            .unwrap_or_else(|error| panic!("{error}"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sync_follows_the_config_flag() {
        let dir = temp_dir("sync");
        let autostart = Autostart::with_dir(&dir);
        autostart
            .sync(true, "flowshot-daemon")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(autostart.is_enabled());
        autostart
            .sync(false, "flowshot-daemon")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!autostart.is_enabled());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enable_rewrites_an_existing_entry() {
        let dir = temp_dir("rewrite");
        let autostart = Autostart::with_dir(&dir);
        autostart
            .enable("old-exec")
            .unwrap_or_else(|e| panic!("{e}"));
        autostart
            .enable("new-exec")
            .unwrap_or_else(|error| panic!("{error}"));
        let contents = std::fs::read_to_string(autostart.entry_path())
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(contents.contains("Exec=new-exec\n"));
        assert!(!contents.contains("old-exec"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exec_values_with_spaces_are_quoted() {
        assert_eq!(
            exec_value(Path::new("/usr/bin/flowshot-daemon")),
            "/usr/bin/flowshot-daemon"
        );
        assert_eq!(
            exec_value(Path::new("/home/a b/flowshot-daemon")),
            "\"/home/a b/flowshot-daemon\""
        );
    }
}
