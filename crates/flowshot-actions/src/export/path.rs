//! Save-path resolution and collision numeration.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use flowshot_core::config::SaveConfig;

use super::pattern::{expand_pattern, sanitize_filename};
use crate::error::ExportError;

/// Resolve the final save path from the configuration.
///
/// # Errors
///
/// Returns [`ExportError`] if the path cannot be resolved.
pub fn resolve_save_path(
    config: &SaveConfig,
    dialog: &dyn super::FileDialogSink,
) -> Result<PathBuf, ExportError> {
    let now = Local::now();
    let pattern = expand_pattern(&config.filename_pattern, now);
    let sanitized = sanitize_filename(&pattern);
    let extension = &config.extension;
    let default_name = format!("{sanitized}.{extension}");

    let base = if config.path.is_empty() {
        match dialog.pick_save_path(&default_name)? {
            Some(p) => p,
            None => return Err(ExportError::Cancelled),
        }
    } else {
        PathBuf::from(&config.path)
    };

    let target = if base.is_dir() {
        base.join(&default_name)
    } else if base.exists() || has_extension(&base) {
        base.with_extension(extension.as_str())
    } else {
        fs::create_dir_all(&base).map_err(|_| ExportError::DirectoryNotFound(base.clone()))?;
        base.join(&default_name)
    };

    Ok(next_available_path(&target))
}

fn has_extension(path: &Path) -> bool {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|n| n.contains('.'))
}

/// Find the next available path by appending `_1`, `_2`, ... before the
/// extension if the target already exists.
#[must_use]
pub fn next_available_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }

    let stem = target
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("");
    let extension = target
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("");
    let parent = target.parent().unwrap_or(Path::new("."));

    let mut counter = 1u32;
    loop {
        let name = if extension.is_empty() {
            format!("{stem}_{counter}")
        } else {
            format!("{stem}_{counter}.{extension}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
        counter = counter.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct MockDialog {
        response: Result<Option<PathBuf>, ExportError>,
    }

    impl super::super::FileDialogSink for MockDialog {
        fn pick_save_path(&self, _default_name: &str) -> Result<Option<PathBuf>, ExportError> {
            match &self.response {
                Ok(Some(p)) => Ok(Some(p.clone())),
                Ok(None) => Ok(None),
                Err(e) => Err(ExportError::Io(std::io::Error::other(e.to_string()))),
            }
        }
    }

    fn unique_tempdir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "flowshot-actions-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
            counter
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn dir_path_appends_pattern_and_extension() {
        let dir = unique_tempdir();
        let config = SaveConfig {
            path: dir.to_string_lossy().into_owned(),
            extension: "png".to_owned(),
            ..SaveConfig::default()
        };
        let dialog = MockDialog { response: Ok(None) };
        let result = resolve_save_path(&config, &dialog);
        assert!(result.is_ok());
        let path = result.unwrap_or_default();
        assert!(path.starts_with(&dir));
        assert!(path.extension().is_some_and(|e| e == "png"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_path_restems_extension() {
        let dir = unique_tempdir();
        let file_path = dir.join("output.bmp");
        let _ = fs::write(&file_path, b"dummy");
        let config = SaveConfig {
            path: file_path.to_string_lossy().into_owned(),
            extension: "png".to_owned(),
            ..SaveConfig::default()
        };
        let dialog = MockDialog { response: Ok(None) };
        let result = resolve_save_path(&config, &dialog);
        assert!(result.is_ok());
        let path = result.unwrap_or_default();
        assert_eq!(path.extension().unwrap_or_default(), "png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_path_uses_dialog() {
        let dir = unique_tempdir();
        let expected = dir.join("chosen.png");
        let config = SaveConfig {
            path: String::new(),
            extension: "png".to_owned(),
            ..SaveConfig::default()
        };
        let dialog = MockDialog {
            response: Ok(Some(expected)),
        };
        let result = resolve_save_path(&config, &dialog);
        assert!(result.is_ok());
        let path = result.unwrap_or_default();
        assert_eq!(path.extension().unwrap_or_default(), "png");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_path_dialog_cancelled_returns_error() {
        let config = SaveConfig {
            path: String::new(),
            ..SaveConfig::default()
        };
        let dialog = MockDialog { response: Ok(None) };
        let result = resolve_save_path(&config, &dialog);
        assert!(matches!(result, Err(ExportError::Cancelled)));
    }

    #[test]
    fn collision_numeration_3x() {
        let dir = unique_tempdir();
        let base = dir.join("shot.png");
        let _ = fs::write(&base, b"0");
        let _ = fs::write(dir.join("shot_1.png"), b"1");
        let _ = fs::write(dir.join("shot_2.png"), b"2");

        let result = next_available_path(&base);
        assert_eq!(result.file_name().unwrap_or_default(), "shot_3.png");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_collision_returns_original() {
        let dir = unique_tempdir();
        let base = dir.join("fresh.png");
        let result = next_available_path(&base);
        assert_eq!(result, base);
        let _ = fs::remove_dir_all(&dir);
    }
}
