//! The todo-23 blur-variant suite: kernel normalization (plan acceptance),
//! the F27 radius clamp, spread, determinism, and the no-op paths.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use flowshot_core::geometry::LogicalPoint;

use super::blur::{BLUR_PASSES, BLUR_RADIUS_MAX, BLUR_RADIUS_MIN, bake_blur, gaussian_kernel};
use super::pixelate::BakeRegion;
use super::tool::FramePixels;

fn region(x: u32, y: u32, w: u32, h: u32) -> BakeRegion {
    BakeRegion { x, y, w, h }
}

fn frame_with(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> FramePixels {
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let at = (y as usize * w as usize + x as usize) * 4;
            rgba[at..at + 3].copy_from_slice(&pixel(x, y));
            rgba[at + 3] = 255;
        }
    }
    FramePixels {
        rgba,
        width: w,
        height: h,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }
}

fn variance(bytes: &[u8]) -> f64 {
    let mean = bytes.iter().map(|b| f64::from(*b)).sum::<f64>() / bytes.len() as f64;
    bytes
        .iter()
        .map(|b| {
            let d = f64::from(*b) - mean;
            d * d
        })
        .sum::<f64>()
        / bytes.len() as f64
}

#[test]
fn kernel_is_normalized_and_symmetric() {
    for sigma in [0.5, 5.0, 6.0, 12.0] {
        let kernel = gaussian_kernel(sigma);
        let sum: f64 = kernel.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12, "sigma {sigma}: sum {sum}");
        let half = kernel.len() / 2;
        for index in 0..half {
            assert_eq!(kernel[index], kernel[kernel.len() - 1 - index]);
        }
        assert_eq!(kernel.len(), 2 * (3.0 * sigma).ceil().max(1.0) as usize + 1);
    }
    // Degenerate sigmas fall back to 1.0 (total function, no panic).
    for sigma in [0.0, -3.0, f64::NAN, f64::INFINITY] {
        let kernel = gaussian_kernel(sigma);
        let sum: f64 = kernel.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
    }
}

#[test]
fn constant_regions_are_invariant_under_the_normalized_kernel() {
    let frame = frame_with(48, 48, |_, _| [91, 91, 91]);
    let out = bake_blur(&frame, region(4, 4, 40, 40), 10).unwrap();
    assert!(
        out.chunks(4)
            .all(|pixel| pixel[..3].iter().all(|c| *c == 91) && pixel[3] == 255),
        "a normalized kernel maps a constant to itself (edge-extend included)"
    );
}

#[test]
fn impulse_energy_spreads_and_the_peak_drops() {
    // A single white pixel on black: after the blur every neighbor within
    // the kernel support carries energy and the peak is reduced (the QA
    // "blur spreads" oracle at the algorithm level).
    let frame = frame_with(64, 64, |x, y| {
        if x == 32 && y == 32 {
            [255, 255, 255]
        } else {
            [0, 0, 0]
        }
    });
    let out = bake_blur(&frame, region(0, 0, 64, 64), 12).unwrap();
    let at = |x: u32, y: u32| -> u8 { out[(y as usize * 64 + x as usize) * 4] };
    assert!(at(32, 32) < 255, "peak reduced, got {}", at(32, 32));
    assert!(at(32, 32) > 0, "energy is conserved locally, not deleted");
    for delta in 1..=2u32 {
        assert!(at(32 + delta, 32) > 0, "spread +{delta}");
        assert!(at(32, 32 + delta) > 0, "spread +{delta}");
    }
    // Beyond the kernel support (2 passes x ceil(3*sigma) = 36) the impulse
    // leaves nothing; monotone falloff lives below the u8 quantization floor
    // at this amplitude, so the shape assert uses the step edge instead.
    assert_eq!(at(10, 32), 0, "outside the kernel support");
}

#[test]
fn step_edge_spreads_monotonically_across_the_kernel_support() {
    // A black->white edge at x=32: the blur must produce a monotone
    // transition ramp (the "blur spreads" oracle at the algorithm level).
    let frame = frame_with(
        64,
        64,
        |x, _| {
            if x < 32 { [0, 0, 0] } else { [255, 255, 255] }
        },
    );
    let out = bake_blur(&frame, region(0, 0, 64, 64), 10).unwrap();
    let at = |x: u32| -> u8 { out[(32usize * 64 + x as usize) * 4] };
    for x in 18..46 {
        assert!(
            at(x) <= at(x + 1),
            "monotone ramp: at({x})={} > at({})={}",
            at(x),
            x + 1,
            at(x + 1)
        );
    }
    assert!(
        at(28) > 0 && at(28) < 128,
        "spread below the edge: {}",
        at(28)
    );
    assert!(
        at(36) > 128 && at(36) < 255,
        "spread above the edge: {}",
        at(36)
    );
    assert_eq!(at(2), 0, "far side stays black (finite support)");
    assert_eq!(at(62), 255, "far side stays white (finite support)");
}

#[test]
fn radius_maps_size_through_the_f27_10_12_clamp() {
    assert_eq!((BLUR_RADIUS_MIN, BLUR_RADIUS_MAX, BLUR_PASSES), (10, 12, 2));
    let frame = frame_with(48, 48, |x, y| [(x * 5) as u8, (y * 5) as u8, 128]);
    let small = bake_blur(&frame, region(4, 4, 40, 40), 1).unwrap();
    let at_min = bake_blur(&frame, region(4, 4, 40, 40), 10).unwrap();
    let at_max = bake_blur(&frame, region(4, 4, 40, 40), 12).unwrap();
    let above = bake_blur(&frame, region(4, 4, 40, 40), 50).unwrap();
    assert_eq!(small, at_min, "sizes below the band clamp to radius 10");
    assert_eq!(at_max, above, "sizes above the band clamp to radius 12");
    assert_ne!(at_min, at_max, "the band endpoints differ");
}

#[test]
fn blur_is_deterministic_and_smooths_high_frequency_content() {
    let frame = frame_with(64, 64, |x, y| {
        if (x / 2 + y / 2) % 2 == 0 {
            [255, 255, 255]
        } else {
            [0, 0, 0]
        }
    });
    let first = bake_blur(&frame, region(8, 8, 48, 48), 11).unwrap();
    let second = bake_blur(&frame, region(8, 8, 48, 48), 11).unwrap();
    assert_eq!(first, second, "byte-identical across runs");
    let rgba = frame.rgba.as_slice();
    let original: Vec<u8> = (8..56)
        .flat_map(|y| {
            (8..56).flat_map(move |x| {
                let at = (y as usize * 64 + x as usize) * 4;
                rgba[at..at + 3].to_vec()
            })
        })
        .collect();
    let blurred: Vec<u8> = first
        .chunks(4)
        .flat_map(|pixel| pixel[..3].to_vec())
        .collect();
    assert!(
        variance(&blurred) < variance(&original) / 4.0,
        "a checkerboard loses almost all contrast: {} vs {}",
        variance(&blurred),
        variance(&original)
    );
}

#[test]
fn empty_region_is_a_noop() {
    let frame = frame_with(8, 8, |_, _| [1, 2, 3]);
    assert!(bake_blur(&frame, region(0, 0, 0, 4), 10).is_none());
    assert!(bake_blur(&frame, region(0, 0, 4, 0), 10).is_none());
    // 1x1 blurs to itself (edge-extend everywhere), no panic.
    let out = bake_blur(&frame, region(3, 3, 1, 1), 10).unwrap();
    assert_eq!(out, vec![1, 2, 3, 255]);
}
