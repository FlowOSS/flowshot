//! Backdrop composition on the real GPU pipeline (plan todo 15).
//!
//! Renders planned [`Backdrop`] scenes offscreen (headless - no window) and
//! reads the pixels back: per-window frozen crops, the dim layer with its
//! even-odd selection cutout, cursor-sprite compositing at the hotspot,
//! rotated-frame upright remapping, and the letterbox placeholder for a
//! missing frame. Flat axis-aligned fills reproduce token bytes exactly (the
//! renderer's documented contract), so interior asserts are pixel-exact.
//!
//! Without a GPU adapter (no Vulkan, not even lavapipe) every test SKIPS
//! with a message - headless CI stays green, this machine runs for real.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::cast_precision_loss
)]

use bytes::BytesMut;
use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::{
    Logical, LogicalPoint, LogicalRect, OutputInfo, PhysicalPoint, PhysicalSize, Transform,
};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::backdrop::{Backdrop, BackdropOptions, CursorSprite, FrozenCapture, PlacedCursor};
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::render::{DisplayList, RenderTarget, Renderer, read_texture_rgba};

struct Gpu {
    _instance: wgpu::Instance,
    ctx: GpuContext,
}

fn gpu_or_skip() -> Option<Gpu> {
    let instance = new_instance();
    match GpuContext::new_headless(&instance) {
        Ok(ctx) => Some(Gpu {
            _instance: instance,
            ctx,
        }),
        Err(error) => {
            eprintln!("SKIP (no usable GPU adapter): {error}");
            None
        }
    }
}

/// Renders one window's backdrop list offscreen and reads it back.
fn compose(
    gpu: &Gpu,
    backdrop: &mut Backdrop,
    index: usize,
    size: (u32, u32),
    options: &BackdropOptions,
) -> Vec<u8> {
    let device = &gpu.ctx.device;
    let queue = &gpu.ctx.queue;
    let mut renderer = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    backdrop
        .upload_for(index, &mut renderer, &gpu.ctx)
        .expect("upload");
    backdrop
        .upload_cursor(&mut renderer, &gpu.ctx)
        .expect("cursor upload");
    let list = backdrop.commands(index, size, options);
    render_list(gpu, &mut renderer, &list, size)
}

fn render_list(
    gpu: &Gpu,
    renderer: &mut Renderer,
    list: &DisplayList,
    size: (u32, u32),
) -> Vec<u8> {
    let device = &gpu.ctx.device;
    let queue = &gpu.ctx.queue;
    let target = renderer
        .create_offscreen_target(device, size.0, size.1)
        .expect("offscreen target");
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render(
            device,
            queue,
            &RenderTarget {
                view: &view,
                width: size.0,
                height: size.1,
            },
            list,
        )
        .expect("render");
    read_texture_rgba(device, queue, &target, size.0, size.1).expect("readback")
}

fn pixel_at(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]
}

fn output(
    connector: &str,
    logical: LogicalRect,
    physical: (i32, i32),
    scale: f64,
    transform: Transform,
) -> OutputInfo {
    OutputInfo::new(
        connector,
        connector,
        logical,
        PhysicalSize::from_raw(physical.0, physical.1),
        scale,
        transform,
    )
    .expect("valid fixture output")
}

fn frame_from(
    connector: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
    transform: Transform,
) -> Frame {
    Frame {
        buffer: FrameBuffer {
            data: BytesMut::from(pixels),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Rgba8888,
        },
        output: OutputRef::Connector(connector.to_owned()),
        scale: 1.0,
        transform,
    }
}

fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let count = usize::try_from(width * height).unwrap();
    let mut data = Vec::with_capacity(count * 4);
    for _ in 0..count {
        data.extend_from_slice(&rgba);
    }
    data
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// HDMI-A-1 8x6 @ 1x at (0,0) red; DP-3 8x6 @ 2x (logical 4x3) at (8,0)
/// green - the mixed-scale stitch fixture (never an averaged scale).
fn dual_capture() -> FrozenCapture {
    FrozenCapture {
        outputs: vec![
            output(
                "HDMI-A-1",
                LogicalRect::from_raw(0.0, 0.0, 8.0, 6.0),
                (8, 6),
                1.0,
                Transform::Normal,
            ),
            output(
                "DP-3",
                LogicalRect::from_raw(8.0, 0.0, 4.0, 3.0),
                (8, 6),
                2.0,
                Transform::Normal,
            ),
        ],
        frames: vec![
            frame_from("HDMI-A-1", 8, 6, &solid(8, 6, RED), Transform::Normal),
            frame_from("DP-3", 8, 6, &solid(8, 6, GREEN), Transform::Normal),
        ],
        cursor: None,
    }
}

fn plan(capture: FrozenCapture) -> Backdrop {
    Backdrop::plan(capture, &DesignTokens::default())
}

#[test]
fn each_window_composes_its_own_frozen_crop() {
    let Some(gpu) = gpu_or_skip() else { return };
    let mut backdrop = plan(dual_capture());
    let options = BackdropOptions {
        dim: false,
        ..Default::default()
    };
    let left = compose(&gpu, &mut backdrop, 0, (8, 6), &options);
    let right = compose(&gpu, &mut backdrop, 1, (8, 6), &options);
    for x in 0..8 {
        for y in 0..6 {
            assert_eq!(pixel_at(&left, 8, x, y), RED, "left window ({x},{y})");
            assert_eq!(pixel_at(&right, 8, x, y), GREEN, "right window ({x},{y})");
        }
    }
}

#[test]
fn dim_darkens_outside_the_selection_cutout_only() {
    let Some(gpu) = gpu_or_skip() else { return };
    let mut backdrop = plan(dual_capture());
    let options = BackdropOptions {
        // Logical (2,1) size 3x2 -> identical physical cutout at scale 1.
        selection: Some(LogicalRect::new(
            Logical(2.0),
            Logical(1.0),
            Logical(3.0),
            Logical(2.0),
        )),
        ..Default::default()
    };
    let pixels = compose(&gpu, &mut backdrop, 0, (8, 6), &options);
    // Inside the cutout: the frozen frame untouched, pixel-exact.
    assert_eq!(pixel_at(&pixels, 8, 3, 2), RED);
    // Outside: dimmed - red pulled down, the contrast token's blue pulled up.
    let outside = pixel_at(&pixels, 8, 0, 0);
    assert!(outside[0] < 255, "dimmed red channel: {outside:?}");
    assert!(outside[2] > 0, "dimmed blue channel: {outside:?}");
    assert_eq!(outside[3], 255, "backdrop stays opaque");
}

#[test]
fn cursor_sprite_composites_at_its_resolved_position() {
    let Some(gpu) = gpu_or_skip() else { return };
    let mut capture = dual_capture();
    capture.cursor = Some(PlacedCursor {
        sprite: CursorSprite {
            rgba: solid(2, 2, [255, 255, 255, 255]),
            width: 2,
            height: 2,
            hotspot: PhysicalPoint::from_raw(1, 1),
        },
        // Global logical (9,1) inside DP-3 (origin 8,0 scale 2): local
        // physical (2,2) - hotspot (1,1) -> top-left (1,1).
        position: LogicalPoint::from_raw(9.0, 1.0),
    });
    let mut backdrop = plan(capture);
    let options = BackdropOptions {
        dim: false,
        ..Default::default()
    };
    let right = compose(&gpu, &mut backdrop, 1, (8, 6), &options);
    for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
        assert_eq!(
            pixel_at(&right, 8, x, y),
            [255, 255, 255, 255],
            "cursor pixel ({x},{y})"
        );
    }
    assert_eq!(pixel_at(&right, 8, 0, 0), GREEN, "frame around the cursor");
    assert_eq!(pixel_at(&right, 8, 3, 3), GREEN);
    // The non-owning window shows no cursor.
    let left = compose(&gpu, &mut backdrop, 0, (8, 6), &options);
    assert_eq!(pixel_at(&left, 8, 1, 1), RED);
}

#[test]
fn missing_frame_window_shows_the_token_placeholder() {
    let Some(gpu) = gpu_or_skip() else { return };
    let mut capture = dual_capture();
    capture.frames.pop();
    let mut backdrop = plan(capture);
    assert_eq!(backdrop.missing().count(), 1);
    let options = BackdropOptions {
        dim: false,
        ..Default::default()
    };
    let right = compose(&gpu, &mut backdrop, 1, (8, 6), &options);
    // The placeholder is the contrast token (#0F172A) - opaque flat fills
    // reproduce token bytes exactly.
    let contrast = [0x0F, 0x17, 0x2A, 255];
    for x in 0..8 {
        for y in 0..6 {
            assert_eq!(pixel_at(&right, 8, x, y), contrast, "({x},{y})");
        }
    }
}

#[test]
fn rotated_output_composes_upright() {
    let Some(gpu) = gpu_or_skip() else { return };
    // Native 2x3 buffer, Rot90 -> upright 3x2: map_point sends native
    // (0, y) to upright (y, 1), so the red left column becomes the bottom
    // row of the composed window.
    let mut native = Vec::new();
    for _y in 0..3 {
        for x in 0..2 {
            native.extend_from_slice(if x == 0 { &RED } else { &GREEN });
        }
    }
    let capture = FrozenCapture {
        outputs: vec![output(
            "ROT-1",
            LogicalRect::from_raw(0.0, 0.0, 3.0, 2.0),
            (2, 3),
            1.0,
            Transform::Rot90,
        )],
        frames: vec![frame_from("ROT-1", 2, 3, &native, Transform::Rot90)],
        cursor: None,
    };
    let mut backdrop = plan(capture);
    let options = BackdropOptions {
        dim: false,
        ..Default::default()
    };
    let pixels = compose(&gpu, &mut backdrop, 0, (3, 2), &options);
    for x in 0..3 {
        assert_eq!(pixel_at(&pixels, 3, x, 0), GREEN, "top row ({x},0)");
        assert_eq!(pixel_at(&pixels, 3, x, 1), RED, "bottom row ({x},1)");
    }
}
