//! The editor-side font-system seam.
//!
//! cosmic-text editing operations need a [`FontSystem`] (shaping after every
//! mutation), but tool instances are created fresh per stroke by `fn`-pointer
//! factories that cannot capture shared state - so the editor borrows the
//! process-wide store in [`crate::render::fonts`], the same one the render paths
//! use, which is built on first use rather than per tool instance.
//!
//! Re-entrancy contract: the store hands out ONE borrow at a time per thread;
//! session methods call through at their top level and never nest (a nested call
//! finds the cell already borrowed, logs, and returns the caller's fallback - lib
//! code never panics).

use cosmic_text::FontSystem;

use crate::render::with_font_system as store;

/// Runs `run` with the shared [`FontSystem`]; `fallback` when the store is
/// already borrowed (the re-entrancy guard above - unreachable by design, logged
/// loudly if it ever fires).
pub(super) fn with_font_system<R>(run: impl FnOnce(&mut FontSystem) -> R, fallback: R) -> R {
    store(run, fallback)
}
