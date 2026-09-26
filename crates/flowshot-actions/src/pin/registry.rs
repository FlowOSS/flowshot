//! The multi-pin registry (plan todo 30 -> todo 32 lifecycle).
//!
//! The daemon's smart lifecycle persists while ANY persistence reason
//! holds; "pins alive" is this registry being non-empty (the replacement
//! for Flameshot's dropped `autoCloseIdleDaemon` flag, Amendment #3). The
//! hosting process registers a pin when its window spawns and unregisters
//! it when the window closes (the UI's `PinActionSink::pin_closed`
//! callback is the bridge).

use std::collections::HashMap;
use std::time::SystemTime;

/// One live pin tracked by the host process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRecord {
    /// Host-assigned pin id (matches `flowshot_ui::pins::PinId::raw()`).
    pub id: u64,
    /// Image width in physical pixels at registration time.
    pub width: u32,
    /// Image height in physical pixels at registration time.
    pub height: u32,
    /// When the pin window opened.
    pub opened_at: SystemTime,
}

/// The set of pins currently alive in this process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PinRegistry {
    records: HashMap<u64, PinRecord>,
}

impl PinRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracks `record`; re-registering an id replaces the old record.
    pub fn register(&mut self, record: PinRecord) {
        self.records.insert(record.id, record);
    }

    /// Stops tracking `id`; returns the removed record when it was live.
    pub fn unregister(&mut self, id: u64) -> Option<PinRecord> {
        self.records.remove(&id)
    }

    /// The live pin with `id`, when tracked.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&PinRecord> {
        self.records.get(&id)
    }

    /// Number of live pins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no pins are alive - the todo-32 daemon treats `false` as a
    /// persistence reason ("pins alive").
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Every live pin id, in ascending order (deterministic for tests and
    /// D-Bus reporting).
    #[must_use]
    pub fn ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.records.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u64) -> PinRecord {
        PinRecord {
            id,
            width: 100,
            height: 80,
            opened_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn register_unregister_tracks_liveness() {
        let mut registry = PinRegistry::new();
        assert!(registry.is_empty());
        registry.register(record(7));
        registry.register(record(3));
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.ids(), vec![3, 7]);
        assert_eq!(registry.get(7), Some(&record(7)));
        assert_eq!(registry.unregister(7), Some(record(7)));
        assert_eq!(registry.unregister(7), None);
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        registry.unregister(3);
        assert!(registry.is_empty());
    }

    #[test]
    fn re_registering_an_id_replaces_the_record() {
        let mut registry = PinRegistry::new();
        registry.register(record(1));
        let updated = PinRecord {
            width: 640,
            height: 480,
            ..record(1)
        };
        registry.register(updated.clone());
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(1), Some(&updated));
    }
}
