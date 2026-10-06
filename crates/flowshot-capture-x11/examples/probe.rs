//! Live X11 capability + output probe - needs a running X session (`DISPLAY`).
//!
//! Prints the [`X11Caps`] the capture path branches on (RANDR / XFIXES /
//! MIT-SHM versions), the root window's pixel format gates (depth, image
//! byte order), and every RANDR output with its derived scale, transform,
//! and logical/physical geometry.
//!
//! Run: `cargo run -p flowshot-capture-x11 --example probe`

use flowshot_capture_x11::{X11Connection, outputs, query_caps};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ImageOrder;

type BoxError = Box<dyn std::error::Error>;

fn main() -> Result<(), BoxError> {
    let conn = X11Connection::connect()?;
    let caps = query_caps(conn.conn())?;
    let setup = conn.conn().setup();
    let screen = &setup.roots[conn.screen()];

    println!(
        "DISPLAY: {:?}",
        std::env::var("DISPLAY").unwrap_or_default()
    );
    println!("caps:");
    println!("  randr:  {}", caps.randr);
    println!("  xfixes: {:?}", caps.xfixes.map(|v| v.to_string()));
    println!("  shm:    {:?}", caps.shm.map(|v| v.to_string()));
    println!(
        "  shm fd-passing fast path (>= 1.2): {}",
        caps.shm.is_some_and(|v| v.at_least(1, 2))
    );
    println!("root window:");
    println!(
        "  screen {}: {}x{} px",
        conn.screen(),
        screen.width_in_pixels,
        screen.height_in_pixels
    );
    println!("  depth: {}", screen.root_depth);
    let byte_order = if setup.image_byte_order == ImageOrder::LSB_FIRST {
        "LSBFirst"
    } else {
        "MSBFirst"
    };
    println!("  image byte order: {byte_order}");

    let outputs = outputs(&conn)?;
    println!("outputs ({}):", outputs.len());
    for output in &outputs {
        let logical = output.logical_rect;
        let buffer = output.buffer_size();
        println!(
            "  {}: logical ({}, {} {}x{}) | physical {}x{} | buffer {}x{} | scale {} | transform {:?}",
            output.connector,
            logical.x.0,
            logical.y.0,
            logical.width.0,
            logical.height.0,
            output.physical_size.width.0,
            output.physical_size.height.0,
            buffer.width.0,
            buffer.height.0,
            output.scale,
            output.transform,
        );
    }

    match flowshot_capture_x11::cursor_pos(&conn)? {
        Some(position) => println!(
            "cursor (global logical): ({}, {})",
            position.x.0, position.y.0
        ),
        None => println!("cursor: inside no output"),
    }
    Ok(())
}
