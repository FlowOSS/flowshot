//! JSONL upload history with bounded capacity.
//!
//! Each successful upload appends one JSON line to the history file.
//! When the file exceeds [`UploadHistory::max_entries`], the oldest
//! entries are evicted (file is rewritten without them). The history
//! stores the delete token so the user can revoke past uploads.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::UploadError;

/// One entry in the upload history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadRecord {
    /// Public URL of the uploaded image.
    pub url: String,
    /// Delete token (Imgur `deletehash`).
    pub delete_hash: String,
    /// When the upload occurred (UTC).
    pub timestamp: DateTime<Utc>,
    /// Original filename at upload time.
    pub filename: String,
}

/// Bounded JSONL upload history.
#[derive(Debug)]
pub struct UploadHistory {
    path: PathBuf,
    max_entries: u32,
}

impl UploadHistory {
    /// Open (or create) a history file at `path` with the given capacity.
    ///
    /// # Errors
    ///
    /// Returns [`UploadError::Io`] if the parent directory cannot be
    /// created.
    pub fn open(path: impl Into<PathBuf>, max_entries: u32) -> Result<Self, UploadError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self { path, max_entries })
    }

    /// Read all records from the history file (oldest first).
    ///
    /// # Errors
    ///
    /// Returns [`UploadError::History`] if a line cannot be parsed.
    pub fn read_all(&self) -> Result<Vec<UploadRecord>, UploadError> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(UploadError::Io(e)),
        };
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for (line_num, line) in reader.lines().enumerate() {
            let line = line.map_err(UploadError::Io)?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: UploadRecord = serde_json::from_str(trimmed)
                .map_err(|e| UploadError::History(format!("line {}: {e}", line_num + 1)))?;
            records.push(record);
        }
        Ok(records)
    }

    /// Append a record, evicting the oldest entries if over capacity.
    ///
    /// # Errors
    ///
    /// Returns [`UploadError::Io`] if the file cannot be written,
    /// [`UploadError::History`] if existing records cannot be parsed.
    pub fn append(&self, record: &UploadRecord) -> Result<(), UploadError> {
        let mut records = self.read_all()?;
        records.push(record.clone());
        self.write_all(&records)
    }

    /// Write the full record list, evicting oldest if over capacity.
    fn write_all(&self, records: &[UploadRecord]) -> Result<(), UploadError> {
        let capped = if records.len() > usize::try_from(self.max_entries).unwrap_or(usize::MAX) {
            let start = records.len() - usize::try_from(self.max_entries).unwrap_or(usize::MAX);
            &records[start..]
        } else {
            records
        };

        let mut file = std::fs::File::create(&self.path)?;
        for record in capped {
            let line = serde_json::to_string(record)
                .map_err(|e| UploadError::History(format!("serialize: {e}")))?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    /// Path to the history file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Maximum number of entries kept.
    #[must_use]
    pub fn max_entries(&self) -> u32 {
        self.max_entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "flowshot-actions-upload-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn record(url: &str, hash: &str) -> UploadRecord {
        UploadRecord {
            url: url.to_owned(),
            delete_hash: hash.to_owned(),
            timestamp: chrono::Utc::now(),
            filename: "test.png".to_owned(),
        }
    }

    #[test]
    fn empty_history_returns_empty_vec() -> Result<(), UploadError> {
        let dir = unique_tempdir();
        let path = dir.join("history.jsonl");
        let history = UploadHistory::open(&path, 25)?;
        let records = history.read_all()?;
        assert!(records.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn append_and_read_roundtrip() -> Result<(), UploadError> {
        let dir = unique_tempdir();
        let path = dir.join("history.jsonl");
        let history = UploadHistory::open(&path, 25)?;
        history.append(&record("https://example.com/1.png", "hash1"))?;
        history.append(&record("https://example.com/2.png", "hash2"))?;
        let records = history.read_all()?;
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].url, "https://example.com/1.png");
        assert_eq!(records[1].delete_hash, "hash2");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn cap_evicts_oldest() -> Result<(), UploadError> {
        let dir = unique_tempdir();
        let path = dir.join("history.jsonl");
        let history = UploadHistory::open(&path, 3)?;
        for i in 0..5 {
            history.append(&record(
                &format!("https://example.com/{i}.png"),
                &format!("hash{i}"),
            ))?;
        }
        let records = history.read_all()?;
        assert_eq!(records.len(), 3);
        // Oldest (0, 1) evicted; newest (2, 3, 4) remain.
        assert_eq!(records[0].url, "https://example.com/2.png");
        assert_eq!(records[2].url, "https://example.com/4.png");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn cap_25_evicts_oldest() -> Result<(), UploadError> {
        let dir = unique_tempdir();
        let path = dir.join("history.jsonl");
        let history = UploadHistory::open(&path, 25)?;
        for i in 0..30 {
            history.append(&record(
                &format!("https://example.com/{i}.png"),
                &format!("hash{i}"),
            ))?;
        }
        let records = history.read_all()?;
        assert_eq!(records.len(), 25);
        assert_eq!(records[0].url, "https://example.com/5.png");
        assert_eq!(records[24].url, "https://example.com/29.png");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
