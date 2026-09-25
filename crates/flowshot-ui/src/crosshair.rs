//! Custom crosshair: the system cursor is hidden
//! (`Window::set_cursor_visible(false)`), so `FlowShot` draws its own - the
//! #1659-class invisibility fix: never rely on compositor cursors.
//!
//! This is the functional todo-13 crosshair (two 1-px full-window arms via a
//! `LineList` pipeline); todo 14's renderer replaces the visuals while the
//! cursor-tracking contract stays. The vertex math is pure and unit-tested;
//! only the pipeline touches wgpu.

use std::borrow::Cow;

/// Fallback crosshair color (opaque white) when the token accent fails to
/// parse; visible on both light and dark backdrops. Todo 14 enforces
/// token-only colors in the renderer.
pub(crate) const FALLBACK_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Parses a `#RRGGBB` design-token color into premultiplied-alpha RGBA
/// floats.
pub(crate) fn parse_srgb_hex(hex: &str) -> Option<[f32; 4]> {
    let digits = hex.strip_prefix('#')?.as_bytes();
    if digits.len() != 6 {
        return None;
    }
    let mut channels = [0u8; 3];
    for (index, channel) in channels.iter_mut().enumerate() {
        let [hi, lo] = digits.get(index * 2..index * 2 + 2)? else {
            return None;
        };
        *channel = from_hex_digit(*hi)? * 16 + from_hex_digit(*lo)?;
    }
    Some([
        f32::from(channels[0]) / 255.0,
        f32::from(channels[1]) / 255.0,
        f32::from(channels[2]) / 255.0,
        1.0,
    ])
}

fn from_hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// The four crosshair arm endpoints in normalized device coordinates for a
/// cursor at surface-local physical `(cursor_x, cursor_y)` on a
/// `width` x `height` surface.
///
/// Vertex order: vertical arm bottom, vertical arm top, horizontal arm left,
/// horizontal arm right (`LineList` pairs). Returns `None` for degenerate or
/// non-finite input. Coordinates outside `[-1, 1]` (cursor beyond the
/// surface during a spanning drag) are returned as-is; the rasterizer clips.
pub(crate) fn crosshair_vertices(
    cursor_x: f64,
    cursor_y: f64,
    width: f64,
    height: f64,
) -> Option<[[f32; 2]; 4]> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    if !cursor_x.is_finite() || !cursor_y.is_finite() {
        return None;
    }
    let nx = to_ndc(2.0 * cursor_x / width - 1.0);
    let ny = to_ndc(1.0 - 2.0 * cursor_y / height);
    Some([[nx, -1.0], [nx, 1.0], [-1.0, ny], [1.0, ny]])
}

/// NDC in `f32` far exceeds raster precision needs; the narrowing is
/// deliberate.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn to_ndc(value: f64) -> f32 {
    value as f32
}

const CROSSHAIR_WGSL: &str = r"
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Surface alpha mode is PreMultiplied; the color is written premultiplied.
    return in.color;
}
";

const VERTEX_ATTRIBUTES: &[wgpu::VertexAttribute] =
    &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

/// Interleaved `[position xy, color rgba]` per vertex.
const VERTEX_STRIDE: u64 = 6 * std::mem::size_of::<f32>() as u64;
/// Two arms, two endpoints each.
const VERTEX_COUNT: u32 = 4;

/// The crosshair render pipeline and its per-frame vertex staging buffer.
#[derive(Debug)]
pub(crate) struct CrosshairPipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    color: [f32; 4],
}

impl CrosshairPipeline {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, color: [f32; 4]) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crosshair.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(CROSSHAIR_WGSL)),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crosshair-pipeline-layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crosshair-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: VERTEX_ATTRIBUTES,
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crosshair-vertices"),
            size: u64::from(VERTEX_COUNT) * VERTEX_STRIDE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            vertex_buffer,
            color,
        }
    }

    pub(crate) fn write_vertices(&self, queue: &wgpu::Queue, vertices: [[f32; 2]; 4]) {
        let mut interleaved = [[0.0f32; 6]; 4];
        for (slot, vertex) in interleaved.iter_mut().enumerate() {
            let Some(position) = vertices.get(slot) else {
                continue;
            };
            vertex[0] = position[0];
            vertex[1] = position[1];
            vertex[2..6].copy_from_slice(&self.color);
        }
        queue.write_buffer(&self.vertex_buffer, 0, &vertices_bytes(&interleaved));
    }

    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..VERTEX_COUNT, 0..1);
    }
}

/// Little-endian byte view of plain-`f32` vertex data (allocation is 96
/// bytes per drawn frame; `bytemuck` is deliberately not a dependency).
fn vertices_bytes(interleaved: &[[f32; 6]]) -> Vec<u8> {
    let stride = usize::try_from(VERTEX_STRIDE).unwrap_or(24);
    let mut bytes = Vec::with_capacity(interleaved.len() * stride);
    for vertex in interleaved {
        for component in vertex {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const TOL: f32 = 1e-6;

    #[test]
    fn vertices_center_cursor_maps_to_ndc_origin() {
        let vertices = crosshair_vertices(960.0, 540.0, 1920.0, 1080.0).unwrap();
        assert!((vertices[0][0] - 0.0).abs() <= TOL);
        assert!((vertices[2][1] - 0.0).abs() <= TOL);
        // Arms span the full surface.
        assert_eq!(vertices[0], [vertices[0][0], -1.0]);
        assert_eq!(vertices[1], [vertices[1][0], 1.0]);
        assert_eq!(vertices[2], [-1.0, vertices[2][1]]);
        assert_eq!(vertices[3], [1.0, vertices[3][1]]);
    }

    #[test]
    fn vertices_corner_cursors_map_to_ndc_corners() {
        let top_left = crosshair_vertices(0.0, 0.0, 1920.0, 1080.0).unwrap();
        assert!((top_left[0][0] - (-1.0)).abs() <= TOL);
        assert!((top_left[2][1] - 1.0).abs() <= TOL);
        let bottom_right = crosshair_vertices(1920.0, 1080.0, 1920.0, 1080.0).unwrap();
        assert!((bottom_right[0][0] - 1.0).abs() <= TOL);
        assert!((bottom_right[2][1] - (-1.0)).abs() <= TOL);
    }

    #[test]
    fn vertices_beyond_surface_are_not_clamped() {
        // Spanning drag: the rasterizer clips, the math stays linear.
        let vertices = crosshair_vertices(2220.0, 500.0, 1920.0, 1080.0).unwrap();
        assert!(vertices[0][0] > 1.0);
    }

    #[test]
    fn vertices_reject_degenerate_input() {
        assert!(crosshair_vertices(10.0, 10.0, 0.0, 100.0).is_none());
        assert!(crosshair_vertices(10.0, 10.0, 100.0, -1.0).is_none());
        assert!(crosshair_vertices(f64::NAN, 10.0, 100.0, 100.0).is_none());
    }

    #[test]
    fn parse_srgb_hex_accepts_token_accent() {
        let color = parse_srgb_hex("#2AA198").unwrap();
        assert!((color[0] - f32::from(0x2Au8) / 255.0).abs() <= TOL);
        assert!((color[1] - f32::from(0xA1u8) / 255.0).abs() <= TOL);
        assert!((color[2] - f32::from(0x98u8) / 255.0).abs() <= TOL);
        assert_eq!(color[3], 1.0);
        assert!(parse_srgb_hex("#2aa198").is_some());
    }

    #[test]
    fn parse_srgb_hex_rejects_malformed() {
        for bad in ["2AA198", "#GGG", "#2AA19", "#2AA1988", "", "#12345g"] {
            assert!(parse_srgb_hex(bad).is_none(), "{bad} parsed");
        }
    }

    #[test]
    fn vertices_bytes_is_little_endian_interleaved() {
        let interleaved = [[1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0]];
        let bytes = vertices_bytes(&interleaved);
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[20..24], &6.0f32.to_le_bytes());
    }
}
