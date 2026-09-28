//! The overlay frame builder (plan todo 41 extraction).
//!
//! ONE display-list construction path shared by the live shell
//! (the `render_window` method) and the offscreen QA harnesses (the
//! todo-15 `--verify-offscreen` pattern, the todo-41 QA bundle): backdrop ->
//! grid -> editor -> selection chrome -> editor chrome -> magnifier, in the
//! production paint order, evaluated at a caller-supplied `now` so motion
//! stills are deterministic under a synthetic clock.

use std::time::Instant;

use crate::backdrop::{Backdrop, BackdropOptions};
use crate::editor::{EditorView, MagnifierTexture};
use crate::render::DisplayList;
use crate::router::WindowSlot;
use crate::state::OverlayCore;
use crate::widgets::ICON_ATLAS_ID;

/// One built overlay frame: the display list plus the CPU-built magnifier
/// texture the caller must upload BEFORE rendering the list (a missing
/// texture id draws the magenta placeholder).
#[derive(Debug)]
pub struct OverlayFrame {
    /// The frame's draw commands, in paint order.
    pub list: DisplayList,
    /// The magnifier zoom texture, when the magnifier paints this frame.
    pub magnifier: Option<MagnifierTexture>,
}

/// Builds `slot`'s frame from the core state. `backdrop` supplies the frozen
/// frame textures and options (`None` = the empty overlay); the LIVE
/// selection rect always supersedes the backdrop option (the todo-16 drag
/// contract). The crosshair is NOT part of the list - it rides the surface's
/// vertex-overlay pipeline in the live shell only.
#[must_use]
pub fn build_overlay_frame(
    core: &OverlayCore,
    slot: WindowSlot,
    surface: (u32, u32),
    backdrop: Option<(&Backdrop, &BackdropOptions)>,
    now: Instant,
) -> OverlayFrame {
    let mut list = match (backdrop, core.router().output_index_for(slot)) {
        (Some((backdrop, options)), Some(output_index)) => {
            let options = BackdropOptions {
                selection: core.selection().rect(),
                ..*options
            };
            backdrop.commands(output_index, surface, &options)
        }
        _ => DisplayList::new(),
    };
    if let Some(output) = core.router().output_for(slot) {
        core.editor().paint_grid(&mut list, output);
        let view = EditorView {
            mouse: core.cursor().map(|cursor| cursor.clamped),
            selection: core.selection().rect(),
            modifiers: *core.modifiers(),
        };
        core.editor().paint_into(&mut list, output, view);
        core.selection().paint_into(&mut list, output, now);
        core.chrome().paint_into(
            &mut list,
            core.editor(),
            core.selection(),
            ICON_ATLAS_ID,
            output,
            now,
        );
    }
    let magnifier = magnifier_pass(core, slot, &mut list, surface);
    OverlayFrame { list, magnifier }
}

/// Paints the magnifier (todo 17) into `list` when `slot` owns the cursor
/// track (the crosshair's slot rule); returns the CPU-built zoom texture
/// for upload.
fn magnifier_pass(
    core: &OverlayCore,
    slot: WindowSlot,
    list: &mut DisplayList,
    surface: (u32, u32),
) -> Option<MagnifierTexture> {
    let cursor = core.cursor().filter(|cursor| cursor.slot == slot)?;
    let output = core.router().output_for(slot)?;
    let local = core.router().to_local(slot, cursor.clamped)?;
    let view = crate::editor::MagnifierView {
        surface,
        cursor_local: local,
        cursor_global: cursor.clamped,
    };
    core.editor()
        .paint_magnifier(list, output, view, core.chrome().tokens())
}
