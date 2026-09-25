//! Prints the one-shot global logical cursor position as JSON.
//!
//! Creates an `ext-image-copy-capture-v1` pointer-cursor session per output,
//! waits up to 500 ms for the first `position` event, converts it from
//! source-local physical into global logical layout space, tears the sessions
//! down, and prints `{"x":..,"y":..}` to stdout, exiting 0.
//!
//! Degradation, never failure: when the cursor overlaps no output, the seat has
//! no pointer, the protocol is absent, or the compositor denied
//! cursor-position permission (`Hyprland` `PERMISSION_TYPE_CURSOR_POS`), this
//! prints `null`, warns on stderr, and STILL exits 0 - a missing cursor
//! position must never fail a capture.
//!
//! Usage:
//! - `cursor_pos` - prints the global logical position JSON.
//! - `cursor_pos --image` - captures the cursor image through the embedded
//!   capture session and prints its dimensions, hotspot, and byte length.
//! - `cursor_pos --stream` - prints the first few events from the long-lived
//!   cursor stream (the compositor pushes initial state on session creation, so
//!   a stationary cursor still yields enter/position/hotspot).
//! - `cursor_pos --pretty` - indents the JSON.

use std::process::ExitCode;

use flowshot_capture::CursorEvent;
use flowshot_capture_wayland::IccBackend;
use futures::StreamExt;
use serde_json::json;

/// How many stream events the `--stream` smoke collects before stopping. The
/// compositor pushes enter + position + hotspot when a cursor session is
/// created, so a stationary cursor yields these without any movement.
const STREAM_SMOKE_EVENTS: usize = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pretty = args.iter().any(|arg| arg == "--pretty");
    let backend = IccBackend::new();
    let value = if args.iter().any(|arg| arg == "--image") {
        image_value(backend)
    } else if args.iter().any(|arg| arg == "--stream") {
        stream_value(backend)
    } else {
        position_value(backend)
    };
    let rendered = if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    };
    match rendered {
        Ok(text) => println!("{text}"),
        Err(err) => eprintln!("warning: could not render JSON: {err}"),
    }
    ExitCode::SUCCESS
}

fn position_value(backend: IccBackend) -> serde_json::Value {
    if let Some((x, y)) = futures::executor::block_on(backend.cursor_pos()) {
        json!({ "x": x, "y": y })
    } else {
        warn_unavailable("cursor position");
        json!(null)
    }
}

fn image_value(backend: IccBackend) -> serde_json::Value {
    if let Some(image) = futures::executor::block_on(backend.cursor_image()) {
        json!({
            "width": image.width,
            "height": image.height,
            "hotspot": { "x": image.hotspot.x.0, "y": image.hotspot.y.0 },
            "rgba_bytes": image.rgba.len(),
        })
    } else {
        warn_unavailable("cursor image");
        json!(null)
    }
}

fn stream_value(backend: IccBackend) -> serde_json::Value {
    use flowshot_capture::CaptureBackend;
    let Some(stream) = backend.cursor_events() else {
        warn_unavailable("cursor stream");
        return json!(null);
    };
    let events: Vec<CursorEvent> =
        futures::executor::block_on(stream.take(STREAM_SMOKE_EVENTS).collect());
    if events.is_empty() {
        warn_unavailable("cursor stream events");
    }
    json!(events.iter().map(event_label).collect::<Vec<String>>())
}

fn event_label(event: &CursorEvent) -> String {
    match event {
        CursorEvent::Entered => "entered".to_owned(),
        CursorEvent::Left => "left".to_owned(),
        CursorEvent::Moved { position } => {
            format!("moved({:.0},{:.0})", position.x.0, position.y.0)
        }
        CursorEvent::Hotspot { offset } => format!("hotspot({},{})", offset.x.0, offset.y.0),
    }
}

fn warn_unavailable(what: &str) {
    eprintln!(
        "warning: {what} unavailable (denied, off-screen, or no \
         ext-image-copy-capture cursor session); printing null"
    );
}
