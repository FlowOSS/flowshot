//! Pure, headless-testable pieces of the X11 selection protocol: the
//! `TARGETS` composition, the servable-name expansion, the INCR chunk
//! arithmetic, and the `MULTIPLE` pair-list decode. No connection, no
//! thread, no environment - every function here is unit-tested without
//! an X server.

use std::ops::Range;

use x11rb::protocol::xproto::Atom;

use super::super::offer::MIME_TEXT_PLAIN;

/// Headroom between the server's maximum request size and the INCR
/// chunk budget (`ChangeProperty` header plus safety margin).
const REQUEST_OVERHEAD: usize = 1024;
/// Upper bound for one INCR chunk, below the protocol maximum: some
/// requestors mishandle oversized property changes.
const INCR_CHUNK_CAP: usize = 256 * 1024;

/// Automatic text variants served alongside `text/plain` (parity with
/// the Wayland backend, whose `wl-clipboard-rs` adds the same aliases).
const TEXT_ALIASES: [&str; 4] = ["text/plain;charset=utf-8", "UTF8_STRING", "STRING", "TEXT"];
/// ICCCM protocol targets every selection owner must serve.
pub(super) const PROTOCOL_TARGETS: [&str; 3] = ["TARGETS", "TIMESTAMP", "MULTIPLE"];

/// The target names one MIME entry is servable under: the MIME itself
/// plus, for `text/plain`, the automatic text variants.
pub(super) fn servable_names(mime: &str) -> Vec<&str> {
    let mut names = vec![mime];
    if mime == MIME_TEXT_PLAIN {
        names.extend(TEXT_ALIASES);
    }
    names
}

/// The full `TARGETS` list for an offer with these MIME types: protocol
/// targets first, then every servable target, deduplicated, in offer
/// order. The single source of truth for the composition:
/// [`super::wire::build_offer`] interns exactly this list (minus the
/// protocol names) into the served targets, and
/// [`super::OwnerState::write_targets`] answers with the protocol atoms
/// followed by those served atoms - the same order by construction.
pub(super) fn target_names<'a>(mimes: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    for name in PROTOCOL_TARGETS {
        push_unique(&mut out, name);
    }
    for mime in mimes {
        for name in servable_names(mime) {
            push_unique(&mut out, name);
        }
    }
    out
}

/// Append `name` unless already present (the `TARGETS` dedup rule).
fn push_unique<'a>(list: &mut Vec<&'a str>, name: &'a str) {
    if !list.contains(&name) {
        list.push(name);
    }
}

/// The INCR chunk budget for a server whose maximum request size is
/// `max_request_bytes`: the request size minus [`REQUEST_OVERHEAD`],
/// capped at [`INCR_CHUNK_CAP`], never zero.
pub(super) fn incr_chunk_size(max_request_bytes: usize) -> usize {
    max_request_bytes
        .saturating_sub(REQUEST_OVERHEAD)
        .clamp(1, INCR_CHUNK_CAP)
}

/// The next INCR slice for a transfer at `offset` of `total` bytes, and
/// whether the transfer is finished: once `offset` reaches `total`, the
/// slice is empty — the ICCCM terminator written after the requestor
/// deleted the final data chunk.
pub(super) fn next_incr_slice(offset: usize, chunk: usize, total: usize) -> (Range<usize>, bool) {
    if offset >= total {
        return (0..0, true);
    }
    let end = (offset + chunk).min(total);
    (offset..end, false)
}

/// Parse a `MULTIPLE` pair-list property value into (target, property)
/// atom pairs; a truncated tail is ignored.
pub(super) fn atom_pairs(value: &[u8]) -> Vec<(Atom, Atom)> {
    value
        .as_chunks::<8>()
        .0
        .iter()
        .map(|[t0, t1, t2, t3, d0, d1, d2, d3]| {
            (
                u32::from_ne_bytes([*t0, *t1, *t2, *t3]),
                u32::from_ne_bytes([*d0, *d1, *d2, *d3]),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn target_names_image_offer_lists_protocol_targets_and_mime() {
        assert_eq!(
            target_names(&["image/png"]),
            ["TARGETS", "TIMESTAMP", "MULTIPLE", "image/png"]
        );
    }

    #[test]
    fn target_names_text_offer_expands_automatic_variants() {
        assert_eq!(
            target_names(&["text/plain"]),
            [
                "TARGETS",
                "TIMESTAMP",
                "MULTIPLE",
                "text/plain",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "STRING",
                "TEXT"
            ]
        );
    }

    #[test]
    fn target_names_combined_offer_keeps_offer_order() {
        assert_eq!(
            target_names(&["image/png", "image/jpeg", "text/plain", "text/uri-list"]),
            [
                "TARGETS",
                "TIMESTAMP",
                "MULTIPLE",
                "image/png",
                "image/jpeg",
                "text/plain",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "STRING",
                "TEXT",
                "text/uri-list"
            ]
        );
    }

    #[test]
    fn target_names_deduplicates_explicit_alias_entries() {
        let targets = target_names(&["text/plain", "UTF8_STRING", "text/plain"]);
        assert_eq!(targets.iter().filter(|t| **t == "UTF8_STRING").count(), 1);
        assert_eq!(targets.iter().filter(|t| **t == "text/plain").count(), 1);
    }

    #[test]
    fn servable_names_expands_only_text_plain() {
        assert_eq!(servable_names("image/png"), ["image/png"]);
        assert_eq!(servable_names("text/uri-list"), ["text/uri-list"]);
        assert_eq!(servable_names("text/plain").len(), 1 + TEXT_ALIASES.len());
    }

    #[test]
    fn incr_chunk_size_caps_at_256kib() {
        assert_eq!(incr_chunk_size(16 * 1024 * 1024), INCR_CHUNK_CAP);
        assert_eq!(incr_chunk_size(INCR_CHUNK_CAP * 2), INCR_CHUNK_CAP);
    }

    #[test]
    fn incr_chunk_size_reserves_overhead_below_cap() {
        assert_eq!(incr_chunk_size(64 * 1024), 64 * 1024 - REQUEST_OVERHEAD);
    }

    #[test]
    fn incr_chunk_size_clamps_degenerate_server_limits() {
        assert_eq!(incr_chunk_size(REQUEST_OVERHEAD), 1);
        assert_eq!(incr_chunk_size(0), 1);
    }

    /// Walk the sender-side INCR state machine: chunks until the payload
    /// is exhausted, then the empty terminator slice.
    fn incr_walk(total: usize, chunk: usize) -> Vec<(Range<usize>, bool)> {
        let mut offset = 0;
        let mut walk = Vec::new();
        loop {
            let (range, finished) = next_incr_slice(offset, chunk, total);
            walk.push((range.clone(), finished));
            if finished {
                return walk;
            }
            offset = range.end;
        }
    }

    #[test]
    fn incr_walk_chunks_remainder_then_terminates() {
        assert_eq!(
            incr_walk(10, 4),
            [(0..4, false), (4..8, false), (8..10, false), (0..0, true)]
        );
    }

    #[test]
    fn incr_walk_exact_multiple_still_terminates_empty() {
        assert_eq!(
            incr_walk(8, 4),
            [(0..4, false), (4..8, false), (0..0, true)]
        );
    }

    #[test]
    fn incr_walk_single_chunk_payload() {
        assert_eq!(incr_walk(3, 4), [(0..3, false), (0..0, true)]);
    }

    #[test]
    fn atom_pairs_parses_target_property_tuples() {
        let mut value = Vec::new();
        for atom in [7u32, 8, 9, 10] {
            value.extend_from_slice(&atom.to_ne_bytes());
        }
        assert_eq!(atom_pairs(&value), [(7, 8), (9, 10)]);
    }

    #[test]
    fn atom_pairs_ignores_truncated_tail() {
        assert_eq!(atom_pairs(&[0, 1, 2]), [] as [(Atom, Atom); 0]);
        assert_eq!(atom_pairs(&[]), [] as [(Atom, Atom); 0]);
    }
}
