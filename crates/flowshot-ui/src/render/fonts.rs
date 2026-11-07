//! The process-wide font database, built on first use.
//!
//! `FontSystem::new()` scans the system font DB, which is a real per-open cost:
//! building one per renderer made a multi-window overlay child pay the scan once
//! per window, and pay it even when the child never shapes a glyph. The store is
//! created on the first shaping call and shared by construction - no renderer,
//! presenter, or editor tool owns an instance, so a 2-window child pays the scan
//! once and a text-free child pays it never.
//!
//! Re-entrancy contract, carried over from the editor seam this replaces:
//! [`with_font_system`] hands out ONE borrow at a time per thread; callers invoke
//! it at their top level and never nest (a nested call finds the cell already
//! borrowed, logs, and returns the caller's fallback - lib code never panics).

use std::cell::RefCell;

use cosmic_text::FontSystem;

thread_local! {
    static STORE: RefCell<Option<FontSystem>> = const { RefCell::new(None) };
}

/// Runs `run` with the shared [`FontSystem`], building the store on first use;
/// `fallback` when the cell is already borrowed (the re-entrancy guard above -
/// unreachable by design, logged loudly if it ever fires).
pub(crate) fn with_font_system<R>(run: impl FnOnce(&mut FontSystem) -> R, fallback: R) -> R {
    match STORE.try_with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .map(|mut store| run(store.get_or_insert_with(FontSystem::new)))
    }) {
        Ok(Some(result)) => result,
        Ok(None) => {
            tracing::error!(
                target: "flowshot_ui::render",
                "font store re-entrant borrow; operation skipped"
            );
            fallback
        }
        // Thread-local destruction already ran (process teardown); no shaping
        // operation is meaningful at that point.
        Err(_) => fallback,
    }
}

/// Whether the store has been built yet - the laziness contract.
#[cfg(test)]
pub(crate) fn loaded() -> bool {
    STORE.with(|cell| cell.borrow().is_some())
}

#[cfg(test)]
mod tests {
    use super::{loaded, with_font_system};

    #[test]
    fn the_store_is_not_built_until_the_first_shape() {
        // The store is per-thread, so a fresh thread is the only way to observe
        // "not built yet" without depending on test order in this binary.
        let outcome = std::thread::spawn(|| {
            let before = loaded();
            with_font_system(|fonts| fonts.db().len(), usize::MAX);
            (before, loaded())
        })
        .join()
        .ok();
        assert_eq!(outcome, Some((false, true)), "laziness is the contract");
    }

    #[test]
    fn one_cell_per_thread_so_every_caller_aliases_the_same_store() {
        // A second instance would let this nested borrow succeed; the same shared
        // cell being already borrowed is what makes the nested call fall back.
        let outcome = std::thread::spawn(|| {
            let nested = with_font_system(
                |_fonts| with_font_system(|inner| inner.db().len(), usize::MAX),
                usize::MAX,
            );
            (nested, loaded())
        })
        .join()
        .ok();
        assert_eq!(
            outcome,
            Some((usize::MAX, true)),
            "the nested caller must get its fallback, and the outer call must have built the store"
        );
    }
}
