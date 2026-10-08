//! The X11 wire helpers of the selection owner: atom interning, the
//! served-target construction (driven by [`super::pure::target_names`],
//! the single composition source of truth), and the two property-write
//! shapes (8-bit byte data, 32-bit atom lists).

use std::sync::Arc;

use x11rb::protocol::xproto::{Atom, ConnectionExt, PropMode, Window};

use super::pure::{PROTOCOL_TARGETS, servable_names, target_names};
use super::{ServedOffer, ServedTarget};
use crate::clipboard::offer::ClipboardOffer;
use crate::error::ClipboardError;

/// Intern one atom by name (created when missing).
pub(super) fn intern<C: ConnectionExt>(conn: &C, name: &[u8]) -> Result<Atom, ClipboardError> {
    Ok(conn
        .intern_atom(false, name)
        .map_err(ClipboardError::transport)?
        .reply()
        .map_err(ClipboardError::transport)?
        .atom)
}

/// Intern every servable target of the offer (MIME atoms plus the
/// automatic text variants), deduplicated, in [`target_names`] order -
/// the composition is the single source of truth shared with the
/// `TARGETS` answer, so the served list can never drift from it.
pub(super) fn build_offer<C: ConnectionExt>(
    conn: &C,
    offer: ClipboardOffer,
) -> Result<ServedOffer, ClipboardError> {
    let mimes: Vec<String> = offer
        .entries
        .iter()
        .map(|entry| entry.mime.clone())
        .collect();
    let mime_refs: Vec<&str> = mimes.iter().map(String::as_str).collect();
    let datas: Vec<Arc<[u8]>> = offer
        .entries
        .into_iter()
        .map(|entry| Arc::from(entry.data))
        .collect();
    let mut targets: Vec<ServedTarget> = Vec::new();
    for name in target_names(&mime_refs) {
        if PROTOCOL_TARGETS.contains(&name) {
            // Protocol targets are answered from `Atoms`, never served
            // from offer data.
            continue;
        }
        let Some(index) = mime_refs
            .iter()
            .position(|mime| servable_names(mime).contains(&name))
        else {
            continue;
        };
        let atom = intern(conn, name.as_bytes())?;
        if targets.iter().any(|t| t.target == atom) {
            continue;
        }
        targets.push(ServedTarget {
            target: atom,
            prop_type: atom,
            data: Arc::clone(&datas[index]),
        });
    }
    Ok(ServedOffer { targets })
}

/// Format-8 `ChangeProperty` (Replace) with error checking.
pub(super) fn change_property8<C: ConnectionExt>(
    conn: &C,
    window: Window,
    property: Atom,
    prop_type: Atom,
    data: &[u8],
) -> Result<(), ClipboardError> {
    let len = u32::try_from(data.len())
        .map_err(|_| ClipboardError::X11("property payload exceeds u32".to_owned()))?;
    conn.change_property(PropMode::REPLACE, window, property, prop_type, 8, len, data)
        .map_err(ClipboardError::transport)?
        .check()
        .map_err(ClipboardError::transport)
}

/// Format-32 `ChangeProperty` (Replace) with error checking.
pub(super) fn change_property32<C: ConnectionExt>(
    conn: &C,
    window: Window,
    property: Atom,
    prop_type: Atom,
    data: &[u32],
) -> Result<(), ClipboardError> {
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let len = u32::try_from(data.len())
        .map_err(|_| ClipboardError::X11("property payload exceeds u32".to_owned()))?;
    conn.change_property(
        PropMode::REPLACE,
        window,
        property,
        prop_type,
        32,
        len,
        &bytes,
    )
    .map_err(ClipboardError::transport)?
    .check()
    .map_err(ClipboardError::transport)
}
