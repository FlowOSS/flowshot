//! Live X11 clipboard smoke test — needs a running X session (`DISPLAY`).
//!
//! Serves offers through [`X11Clipboard`] and reads them back over a
//! second connection with `ConvertSelection`, covering: a small text
//! offer (direct property transfer), the `TARGETS` listing, offer
//! supersede, and a payload above the INCR chunk cap (incremental
//! transfer). Doubles as the QA paste tool on machines without
//! `xclip`/`xsel`.
//!
//! Run: `cargo run -p flowshot-actions --example x11_smoke`
//!
//! Readback mode reads whatever the CURRENT `CLIPBOARD` owner offers
//! (e.g. the flowshot daemon after `capture -c`) from an independent
//! connection — the `xclip -selection clipboard -o` equivalent:
//!
//! ```sh
//! cargo run -p flowshot-actions --example x11_smoke -- read [SAVE_PATH]
//! ```
//!
//! It prints the owner window, the `TARGETS` list, the preferred
//! payload target (`image/png` when offered), the byte count, and for
//! PNG payloads the IHDR dimensions; `SAVE_PATH` optionally writes the
//! payload to disk. A selection with no owner is reported as such
//! (exit 0) — that is the "offer is gone" proof, not a failure.

use std::time::{Duration, Instant};

use flowshot_actions::clipboard::{
    ClipboardBackend, ClipboardOffer, MIME_PNG, MIME_TEXT_PLAIN, OfferEntry, X11Clipboard,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, ConnectionExt, CreateWindowAux, EventMask, Property, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

type BoxError = Box<dyn std::error::Error>;

const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const GET_LIMIT: u32 = 0x00FF_FFFF;
const INCR_PAYLOAD_LEN: usize = 1024 * 1024;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("smoke") => run(),
        Some("read") => read_clipboard(args.get(1).map(String::as_str)),
        Some(other) => Err(format!("unknown mode {other:?}; expected `smoke` or `read`").into()),
    };
    match result {
        Ok(report) => {
            println!("{report}");
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("x11 smoke FAILED: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, BoxError> {
    let backend = X11Clipboard::new();
    let (reader, window) = open_reader()?;

    backend.serve(text_offer("flowshot x11 smoke"))?;
    let direct = read_selection(&reader, window, "UTF8_STRING")?;
    assert_eq!(direct, b"flowshot x11 smoke", "direct text round-trip");

    let targets = read_targets(&reader, window)?;
    for required in [
        "TARGETS",
        "TIMESTAMP",
        "MULTIPLE",
        "text/plain",
        "UTF8_STRING",
    ] {
        assert!(
            targets.iter().any(|name| name == required),
            "target {required} missing from {targets:?}"
        );
    }

    // Supersede: a second serve() replaces the first offer; the 1 MiB
    // payload forces the INCR path (chunk cap is 256 KiB).
    let big = "x".repeat(INCR_PAYLOAD_LEN);
    backend.serve(text_offer(&big))?;
    let incr = read_selection(&reader, window, MIME_TEXT_PLAIN)?;
    assert_eq!(incr.len(), big.len(), "INCR payload length");
    assert!(
        incr.iter().all(|byte| *byte == b'x'),
        "INCR payload content"
    );

    Ok(format!(
        "x11 smoke OK: direct text, TARGETS ({} targets), supersede, {} KiB INCR round-trip",
        targets.len(),
        INCR_PAYLOAD_LEN / 1024
    ))
}

/// Read the live `CLIPBOARD` offer from whatever process owns it.
fn read_clipboard(save_path: Option<&str>) -> Result<String, BoxError> {
    let (conn, window) = open_reader()?;
    let clipboard = intern(&conn, "CLIPBOARD")?;
    let owner = conn.get_selection_owner(clipboard)?.reply()?.owner;
    if owner == x11rb::NONE {
        return Ok("clip readback: CLIPBOARD selection has NO OWNER (offer gone)".to_owned());
    }

    let targets = read_targets(&conn, window)?;
    let payload_target = targets
        .iter()
        .find(|name| name.as_str() == MIME_PNG)
        .or_else(|| {
            targets
                .iter()
                .find(|name| !matches!(name.as_str(), "TARGETS" | "TIMESTAMP" | "MULTIPLE"))
        })
        .ok_or("owner offers no payload target")?
        .clone();
    let payload = read_selection(&conn, window, &payload_target)?;
    if let Some(path) = save_path {
        std::fs::write(path, &payload)?;
    }

    let dimensions = png_ihdr_dimensions(&payload)
        .map(|(width, height)| format!(" IHDR {width}x{height}"))
        .unwrap_or_default();
    let saved = save_path
        .map(|path| format!(" saved={path}"))
        .unwrap_or_default();
    Ok(format!(
        "clip readback OK: owner=0x{owner:x} TARGETS={} [{}] target={payload_target} bytes={}{}{}",
        targets.len(),
        targets.join(", "),
        payload.len(),
        dimensions,
        saved,
    ))
}

/// Width/height from a PNG's IHDR chunk: 8-byte signature, 4-byte chunk
/// length, `IHDR`, then big-endian width and height (offsets 16..24).
fn png_ihdr_dimensions(payload: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if payload.get(0..8)? != SIGNATURE || payload.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(payload.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(payload.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

/// Connection plus a 1x1 `INPUT_ONLY` requestor window that receives the
/// selection protocol's `PropertyNotify` events.
fn open_reader() -> Result<(RustConnection, Window), BoxError> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or("display has no screen")?
        .root;
    let window = conn.generate_id()?;
    conn.create_window(
        0,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    Ok((conn, window))
}

fn text_offer(text: &str) -> ClipboardOffer {
    ClipboardOffer {
        entries: vec![OfferEntry::new(MIME_TEXT_PLAIN, text.as_bytes())],
    }
}

/// Request one target from the CLIPBOARD owner and return the payload
/// (transparently reassembling an INCR transfer).
fn read_selection(
    conn: &RustConnection,
    window: Window,
    target: &str,
) -> Result<Vec<u8>, BoxError> {
    let clipboard = intern(conn, "CLIPBOARD")?;
    let target_atom = intern(conn, target)?;
    let property = intern(conn, "FLOWSHOT_SMOKE_PROP")?;
    conn.convert_selection(
        window,
        clipboard,
        target_atom,
        property,
        x11rb::CURRENT_TIME,
    )?;
    conn.flush()?;
    let notified = wait_selection_notify(conn, window)?;
    assert!(
        notified != x11rb::NONE,
        "conversion of {target} was refused"
    );

    let first = get_delete(conn, window, property)?;
    if first.type_ != intern(conn, "INCR")? {
        return Ok(first.value);
    }
    let mut payload = Vec::new();
    loop {
        wait_property_notify(conn, window, property)?;
        let chunk = get_delete(conn, window, property)?;
        if chunk.value.is_empty() {
            return Ok(payload);
        }
        payload.extend_from_slice(&chunk.value);
    }
}

/// Request `TARGETS` and resolve the atom list to names.
fn read_targets(conn: &RustConnection, window: Window) -> Result<Vec<String>, BoxError> {
    let value = read_selection(conn, window, "TARGETS")?;
    value
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| {
            let atom: Atom = u32::from_ne_bytes(*chunk);
            let name = conn.get_atom_name(atom)?.reply()?.name;
            Ok(String::from_utf8_lossy(&name).into_owned())
        })
        .collect()
}

fn intern(conn: &RustConnection, name: &str) -> Result<Atom, BoxError> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn get_delete(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<x11rb::protocol::xproto::GetPropertyReply, BoxError> {
    Ok(conn
        .get_property(true, window, property, x11rb::NONE, 0, GET_LIMIT)?
        .reply()?)
}

fn wait_selection_notify(conn: &RustConnection, window: Window) -> Result<Atom, BoxError> {
    wait_event(conn, "SelectionNotify", |event| match event {
        Event::SelectionNotify(notify) if notify.requestor == window => Some(notify.property),
        _ => None,
    })
}

fn wait_property_notify(
    conn: &RustConnection,
    window: Window,
    property: Atom,
) -> Result<(), BoxError> {
    wait_event(conn, "PropertyNotify", |event| match event {
        Event::PropertyNotify(notify)
            if notify.window == window
                && notify.atom == property
                && notify.state == Property::NEW_VALUE =>
        {
            Some(())
        }
        _ => None,
    })
}

fn wait_event<T>(
    conn: &RustConnection,
    what: &str,
    extract: impl Fn(Event) -> Option<T>,
) -> Result<T, BoxError> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if let Some(event) = conn.poll_for_event()? {
            if let Some(value) = extract(event) {
                return Ok(value);
            }
        } else {
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for {what}").into());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}
