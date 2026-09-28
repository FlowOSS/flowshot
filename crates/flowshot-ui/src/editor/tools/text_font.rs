//! The editor-side font-system seam.
//!
//! cosmic-text editing operations need a [`FontSystem`] (shaping after every
//! mutation), but tool instances are created fresh per stroke by `fn`-pointer
//! factories that cannot capture shared state - so the font database lives
//! in a thread-local, built once per thread (the GPU-side text stack owns its
//! own separate instance; the two never alias).
//!
//! Re-entrancy contract: [`with_font_system`] hands out ONE borrow at a time
//! per thread; session methods call it at their top level and never nest
//! (a nested call would find the cell already borrowed, log, and return the
//! caller's fallback - lib code never panics).

use std::cell::RefCell;

use cosmic_text::FontSystem;

thread_local! {
    static FONT_SYSTEM: RefCell<FontSystem> = RefCell::new(FontSystem::new());
}

/// Runs `run` with this thread's shared [`FontSystem`]; `fallback` when the
/// cell is already borrowed (the re-entrancy guard above - unreachable by
/// design, logged loudly if it ever fires).
pub(super) fn with_font_system<R>(run: impl FnOnce(&mut FontSystem) -> R, fallback: R) -> R {
    match FONT_SYSTEM.try_with(|cell| cell.try_borrow_mut().ok().map(|mut fonts| run(&mut fonts))) {
        Ok(Some(result)) => result,
        Ok(None) => {
            tracing::error!(
                target: "flowshot_ui::editor",
                "font system re-entrant borrow; operation skipped"
            );
            fallback
        }
        // Thread-local destruction already ran (process teardown); no text
        // operation is meaningful at that point.
        Err(_) => fallback,
    }
}
