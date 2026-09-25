#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::format_push_string
)]

use std::env;
use std::fmt::Write;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("icons.rs");
    let atlas_path = Path::new(&out_dir).join("icon_atlas.png");

    let icon_dir = Path::new("icons");
    let mut icons = Vec::new();

    if let Ok(entries) = fs::read_dir(icon_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("svg") {
                let name = path.file_stem().unwrap().to_str().unwrap().to_string();
                icons.push((name, path));
            }
        }
    }

    icons.sort_by(|a, b| a.0.cmp(&b.0));

    let icon_size = 24;
    let padding = 2;
    let cell_size = icon_size + padding * 2;

    let cols = 8;
    let rows = (icons.len() as u32).div_ceil(cols);

    let atlas_width = cols * cell_size;
    let atlas_height = rows * cell_size;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(atlas_width, atlas_height).unwrap();

    let mut rs_code = String::new();
    rs_code.push_str("pub const ICON_SIZE: u32 = 24;\n");
    rs_code.push_str("pub const ATLAS_WIDTH: u32 = ");
    rs_code.push_str(&atlas_width.to_string());
    rs_code.push_str(";\n");
    rs_code.push_str("pub const ATLAS_HEIGHT: u32 = ");
    rs_code.push_str(&atlas_height.to_string());
    rs_code.push_str(";\n\n");

    rs_code.push_str("pub const ICON_ATLAS: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/icon_atlas.png\"));\n\n");

    rs_code.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    rs_code.push_str("pub enum Icon {\n");

    for (i, (name, path)) in icons.iter().enumerate() {
        let svg_data = fs::read_to_string(path).unwrap();
        let svg_data = svg_data.replace("currentColor", "#FFFFFF");
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_data(svg_data.as_bytes(), &opt).unwrap();

        let col = (i as u32) % cols;
        let row = (i as u32) / cols;

        let x = col * cell_size + padding;
        let y = row * cell_size + padding;

        let transform = resvg::tiny_skia::Transform::from_translate(x as f32, y as f32);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let enum_name = name
            .split('-')
            .map(|s| {
                let mut c = s.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<String>();

        rs_code.push_str(&format!("    {enum_name},\n"));
    }

    rs_code.push_str("}\n\n");

    rs_code.push_str("impl Icon {\n");
    rs_code.push_str("    #[allow(clippy::must_use_candidate)]\n");
    rs_code.push_str("    pub fn rect(&self) -> [f32; 4] {\n");
    rs_code.push_str("        match self {\n");

    for (i, (name, _)) in icons.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;

        let x = col * cell_size + padding;
        let y = row * cell_size + padding;

        let enum_name = name
            .split('-')
            .map(|s| {
                let mut c = s.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<String>();

        let _ = writeln!(
            rs_code,
            "            Icon::{enum_name} => [{x}_f32, {y}_f32, {icon_size}_f32, {icon_size}_f32],"
        );
    }

    rs_code.push_str("        }\n");
    rs_code.push_str("    }\n");
    rs_code.push_str("}\n");

    pixmap.save_png(&atlas_path).unwrap();
    fs::write(&dest_path, rs_code).unwrap();
}
