//! The todo-23 secure-pixelate algorithm suite (plan acceptance:
//! determinism, irreversibility, the F27 constants, the 1x1 no-op failure
//! path, the 4K perf gate).
//!
//! The plan's "SIMD path == scalar path byte-identical" acceptance is
//! STRUCTURAL in this task: the dependency freeze (no root Cargo.lock
//! edits) keeps `rayon`/`fast_image_resize` out, so only the scalar path ships;
//! byte-identity-by-construction is secured the way the plan mandates -
//! the noise buffer is pre-generated in canonical pixel order
//! ([`noise_canonical_order_is_x_outer_y_inner_and_seed_stable`]) so any
//! future SIMD consumer reads the exact same deviates.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::time::Instant;

use flowshot_core::geometry::LogicalPoint;

use super::pixelate::noise::{NoiseBuffer, sampling_sigma};
use super::pixelate::{BakeRegion, Fringes, bake_pixelate, grid_size, pixelate_grid};
use super::tool::FramePixels;
use super::undo::EditorUndo;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn region(x: u32, y: u32, w: u32, h: u32) -> BakeRegion {
    BakeRegion { x, y, w, h }
}

/// A synthetic frame from an RGB generator (row-major call order).
fn frame_with(w: u32, h: u32, mut pixel: impl FnMut(u32, u32) -> [u8; 3]) -> FramePixels {
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

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> FramePixels {
    frame_with(w, h, move |_, _| rgb)
}

/// A deterministic LCG (the fixture's pseudo-random interior content).
struct Lcg(u64);
impl Lcg {
    fn next_u8(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u8
    }
}

// ---------------------------------------------------------------------------
// A. The F27 grid formula and the no-op failure paths
// ---------------------------------------------------------------------------

#[test]
fn grid_formula_is_trunc_dim_times_half_over_size_plus_one() {
    // 300x200 @ size 2: trunc(300 * 0.5/3) = 50, trunc(200 * 0.5/3) = 33.
    assert_eq!(grid_size(region(0, 0, 300, 200), 2), Some((50, 33)));
    // size 0: Flameshot guards the DENOMINATOR (qMax(1, size+1) = 1).
    assert_eq!(grid_size(region(0, 0, 300, 200), 0), Some((150, 100)));
    // 11x11 @ size 1: trunc(11 * 0.25) = 2 (partial blocks at the edges).
    assert_eq!(grid_size(region(0, 0, 11, 11), 1), Some((2, 2)));
}

#[test]
fn one_by_one_region_is_a_noop_not_a_panic() {
    // The plan's failure QA: grid collapses to zero -> None, no panic.
    assert_eq!(grid_size(region(0, 0, 1, 1), 2), None);
    let frame = solid(8, 8, [10, 20, 30]);
    assert!(bake_pixelate(&frame, region(3, 3, 1, 1), 2).is_none());
    // 3x3 @ size 2: trunc(3 * 0.5/3) = 0 -> no-op as well.
    assert!(bake_pixelate(&frame, region(2, 2, 3, 3), 2).is_none());
}

// ---------------------------------------------------------------------------
// B. Determinism (plan acceptance: same seed+input -> byte-identical)
// ---------------------------------------------------------------------------

#[test]
fn bakes_are_byte_identical_across_runs() {
    let frame = frame_with(64, 48, |x, y| {
        [(x * 4) as u8, (y * 5) as u8, ((x + y) * 3) as u8]
    });
    let first = bake_pixelate(&frame, region(8, 6, 40, 30), 2).unwrap();
    let second = bake_pixelate(&frame, region(8, 6, 40, 30), 2).unwrap();
    assert_eq!(first, second, "same seed + input must be byte-identical");
    assert_eq!(first.len(), 40 * 30 * 4);
    assert!(first.iter().skip(3).step_by(4).all(|a| *a == 255), "opaque");
}

#[test]
fn noise_canonical_order_is_x_outer_y_inner_and_seed_stable() {
    let a = NoiseBuffer::generate(3, 2, 2);
    let b = NoiseBuffer::generate(3, 2, 2);
    assert_eq!(a, b, "seed 42 is a constant - buffers must match");
    // Canonical order (x outer, y inner): widening the grid by one column
    // APPENDS one column of deviates - the shared prefix is untouched.
    let wide = NoiseBuffer::generate(4, 2, 2);
    for x in 0..3 {
        for y in 0..2 {
            assert_eq!(a.color(x, y), wide.color(x, y));
            assert_eq!(a.sample(x, y, 2), wide.sample(x, y, 2));
        }
    }
    assert_ne!(a.color(0, 0), a.color(1, 1), "deviates vary per pixel");
    assert_eq!(sampling_sigma(2), 11.0, "F27: N(0, 5*size+1)");
}

#[test]
fn golden_grid_fingerprint_pins_the_exact_algorithm() {
    // 16x16 diagonal gradient; region (3,3,10,8) @ size 1 -> 2x2 grid.
    // Pinned +-1 per byte: the Box-Muller transcendentals are per-platform
    // deterministic (the acceptance is per-run) but not cross-libm exact.
    let frame = frame_with(16, 16, |x, y| {
        let v = (x * 8 + y * 4) as u8;
        [v, v.wrapping_mul(3), 255 - v]
    });
    let (grid, gw, gh) = pixelate_grid(&frame, region(3, 3, 10, 8), 1).unwrap();
    assert_eq!((gw, gh), (2, 2));
    let golden: [u8; 16] = [
        86, 86, 213, 255, 37, 143, 186, 255, 57, 117, 188, 255, 45, 89, 128, 255,
    ];
    for (actual, expected) in grid.iter().zip(golden) {
        assert!(
            (i16::from(*actual) - i16::from(expected)).abs() <= 1,
            "grid byte {actual} drifted from golden {expected}"
        );
    }
}

// ---------------------------------------------------------------------------
// C. SECURITY: the interior is never an input (Amendment #3)
// ---------------------------------------------------------------------------

#[test]
fn interior_content_cannot_leak_identical_output_for_any_interior() {
    // Two frames: identical 1px fringe ring around the region, radically
    // different interiors (checkerboard vs its inverse vs random). The
    // secure algorithm reads ONLY the fringes, so the outputs must be
    // byte-identical - there is no residual structure to recover, by
    // construction (stronger than the task's checkerboard-mean criterion).
    // The sampled fringe lines for region (8,8,48,32) are y=7, y=40, x=7,
    // x=56 - everything outside the region interior is uniform gray so the
    // three frames differ ONLY where the algorithm never reads.
    let fringe = |x: u32, y: u32| -> bool { !(8..56).contains(&x) || !(8..40).contains(&y) };
    let checker = |x: u32, y: u32| (x / 2 + y / 2).is_multiple_of(2);
    let mut lcg = Lcg(7);
    let randoms: Vec<u8> = (0..64 * 48 * 3).map(|_| lcg.next_u8()).collect();
    let frames = [
        frame_with(64, 48, |x, y| {
            if fringe(x, y) {
                [90, 90, 90]
            } else if checker(x, y) {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        }),
        frame_with(64, 48, |x, y| {
            if fringe(x, y) {
                [90, 90, 90]
            } else if checker(x, y) {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            }
        }),
        frame_with(64, 48, |x, y| {
            if fringe(x, y) {
                [90, 90, 90]
            } else {
                let at = (y as usize * 64 + x as usize) * 3;
                [randoms[at], randoms[at + 1], randoms[at + 2]]
            }
        }),
    ];
    let baked: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| bake_pixelate(frame, region(8, 8, 48, 32), 2).unwrap())
        .collect();
    assert_eq!(baked[0], baked[1], "checkerboard vs inverse: identical");
    assert_eq!(baked[1], baked[2], "checkerboard vs random: identical");
}

#[test]
fn bicubic_reconstruction_of_the_mosaic_stays_below_20db_psnr() {
    // Plan acceptance: an attacker's best smooth reconstruction (bicubic
    // upscale of the mosaic grid) vs the ORIGINAL interior must land under
    // 20 dB PSNR - the redaction holds. Fixture: pseudo-random interior
    // (maximum reconstructive surprise), uniform fringe.
    let mut lcg = Lcg(4242);
    // Uniform gray fringe ring at 7/56 (the rows/cols the F27 offsets
    // sample for the region 8..56), pseudo-random interior.
    let frame = frame_with(64, 64, |x, y| {
        if x == 7 || x == 56 || y == 7 || y == 56 {
            [128, 128, 128]
        } else {
            [lcg.next_u8(), lcg.next_u8(), lcg.next_u8()]
        }
    });
    let (grid, gw, gh) = pixelate_grid(&frame, region(8, 8, 48, 48), 2).unwrap();
    assert_eq!((gw, gh), (8, 8));
    let reconstruction = bicubic_upscale(&grid, gw, gh, 48, 48);
    let psnr = psnr_against_region(&reconstruction, &frame, region(8, 8, 48, 48));
    assert!(psnr < 20.0, "PSNR {psnr:.2} dB must stay below 20 dB");
}

/// Catmull-Rom bicubic upscale (the attacker's smooth reconstruction).
fn bicubic_upscale(grid: &[u8], gw: u32, gh: u32, w: u32, h: u32) -> Vec<u8> {
    fn sample(grid: &[u8], gw: u32, gh: u32, x: i64, y: i64, c: usize) -> f64 {
        let x = x.clamp(0, i64::from(gw) - 1) as usize;
        let y = y.clamp(0, i64::from(gh) - 1) as usize;
        f64::from(grid[(y * gw as usize + x) * 4 + c])
    }
    fn catmull(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
        0.5 * ((2.0 * p1)
            + (-p0 + p2) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
    }
    let mut out = vec![0u8; w as usize * h as usize * 4];
    for dy in 0..h {
        for dx in 0..w {
            let fx = (f64::from(dx) + 0.5) * f64::from(gw) / f64::from(w) - 0.5;
            let fy = (f64::from(dy) + 0.5) * f64::from(gh) / f64::from(h) - 0.5;
            let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            for c in 0..3 {
                let rows: Vec<f64> = (-1..=2)
                    .map(|oy| {
                        catmull(
                            sample(grid, gw, gh, x0 - 1, y0 + oy, c),
                            sample(grid, gw, gh, x0, y0 + oy, c),
                            sample(grid, gw, gh, x0 + 1, y0 + oy, c),
                            sample(grid, gw, gh, x0 + 2, y0 + oy, c),
                            tx,
                        )
                    })
                    .collect();
                let v = catmull(rows[0], rows[1], rows[2], rows[3], ty);
                out[(dy as usize * w as usize + dx as usize) * 4 + c] =
                    v.round().clamp(0.0, 255.0) as u8;
            }
            out[(dy as usize * w as usize + dx as usize) * 4 + 3] = 255;
        }
    }
    out
}

fn psnr_against_region(reconstruction: &[u8], frame: &FramePixels, at: BakeRegion) -> f64 {
    let mut se = 0.0f64;
    let mut count = 0usize;
    for dy in 0..at.h {
        for dx in 0..at.w {
            let src = ((at.y + dy) as usize * frame.width as usize + (at.x + dx) as usize) * 4;
            let rec = (dy as usize * at.w as usize + dx as usize) * 4;
            for c in 0..3 {
                let d = f64::from(frame.rgba[src + c]) - f64::from(reconstruction[rec + c]);
                se += d * d;
                count += 1;
            }
        }
    }
    let mse = se / count as f64;
    10.0 * (255.0 * 255.0 / mse).log10()
}

// ---------------------------------------------------------------------------
// D. Fringe extraction (the F27 offset rules) and the upscale structure
// ---------------------------------------------------------------------------

#[test]
fn fringes_sit_outside_the_region_except_at_frame_borders() {
    let frame = solid(64, 48, [0, 0, 0]);
    let interior = Fringes::new(&frame, region(10, 10, 20, 15));
    assert_eq!(interior.top_y, 9, "one row ABOVE the region");
    assert_eq!(interior.bottom_y, 25, "y+h-1+1");
    assert_eq!(interior.left_x, 9);
    assert_eq!(interior.right_x, 30);
    let corner = Fringes::new(&frame, region(0, 0, 64, 48));
    assert_eq!(corner.top_y, 0, "frame border -> the region's own edge");
    assert_eq!(corner.left_x, 0);
    assert_eq!(corner.bottom_y, 47);
    assert_eq!(corner.right_x, 63);
}

#[test]
fn upscale_covers_every_pixel_with_uniform_partial_edge_blocks() {
    // 11x11 @ size 1 -> 2x2 grid -> nearest upscale with PARTIAL edge
    // blocks: columns 0..6 and 6..11 (rows likewise). Every pixel inside a
    // block is byte-identical (the QA "block interior uniform" oracle).
    let frame = frame_with(16, 16, |x, y| [x as u8 * 16, y as u8 * 16, 77]);
    let out = bake_pixelate(&frame, region(2, 2, 11, 11), 1).unwrap();
    assert_eq!(out.len(), 11 * 11 * 4);
    let pixel = |dx: u32, dy: u32| -> [u8; 4] {
        let at = (dy as usize * 11 + dx as usize) * 4;
        [out[at], out[at + 1], out[at + 2], out[at + 3]]
    };
    for dy in 0..11 {
        for dx in 0..11 {
            let anchor = |d: u32, grid: u32, dim: u32| -> u32 {
                let block = d * grid / dim;
                (block * dim).div_ceil(grid)
            };
            assert_eq!(
                pixel(dx, dy),
                pixel(anchor(dx, 2, 11), anchor(dy, 2, 11)),
                "({dx},{dy}) must equal its block anchor"
            );
        }
    }
    assert_ne!(pixel(0, 0), pixel(10, 0), "distinct blocks stay distinct");
}

// ---------------------------------------------------------------------------
// E. The unified undo journal (core UndoStack semantics, extended payload)
// ---------------------------------------------------------------------------

#[test]
fn journal_mirrors_the_core_undo_stack_semantics() {
    use flowshot_core::scene::Scene;
    let mut journal = EditorUndo::with_limit(2);
    let snap = |n: u8| {
        (
            Scene::new(),
            vec![super::effect::PixelEffect::new(
                super::effect::EffectKind::Pixelate,
                super::effect::Bake {
                    rect: flowshot_core::geometry::LogicalRect::from_raw(0.0, 0.0, 1.0, 1.0),
                    region: region(0, 0, 1, 1),
                    pixels: vec![n; 4],
                },
            )],
        )
    };
    assert!(!journal.can_undo());
    assert!(journal.undo().is_none(), "silent no-op at history start");
    journal.push(snap(0), snap(1));
    journal.push(snap(1), snap(2));
    assert_eq!(journal.undo_depth(), 2);
    let (scene, effects) = journal.undo().unwrap();
    assert_eq!(scene.object_count(), 0);
    assert_eq!(effects[0].pixels(), &[1, 1, 1, 1], "before snapshot");
    let (_, effects) = journal.redo().unwrap();
    assert_eq!(effects[0].pixels(), &[2, 2, 2, 2], "after snapshot");
    assert!(journal.redo().is_none(), "silent no-op at history end");
    // Push after undo discards the redo tail.
    journal.undo().unwrap();
    journal.push(snap(1), snap(3));
    assert!(!journal.can_redo());
    // Limit evicts the oldest.
    journal.push(snap(3), snap(4));
    assert_eq!(journal.len(), 2);
    assert_eq!(journal.undo_depth(), 2);
    journal.set_limit(1);
    assert_eq!(journal.len(), 1);
    assert_eq!(journal.undo_depth(), 1);
    // Limit 0 disables history.
    let mut disabled = EditorUndo::with_limit(0);
    disabled.push(snap(0), snap(1));
    assert!(disabled.is_empty());
    assert!(!disabled.can_undo());
}

// ---------------------------------------------------------------------------
// F. Perf gate (plan acceptance: 4K-region pixelate < 50ms, log assert)
// ---------------------------------------------------------------------------

#[test]
fn pixelate_4k_region_meets_the_perf_gate() {
    let mut lcg = Lcg(99);
    let frame = frame_with(3840, 2160, |_, _| {
        [lcg.next_u8(), lcg.next_u8(), lcg.next_u8()]
    });
    let started = Instant::now();
    let out = bake_pixelate(&frame, region(0, 0, 3840, 2160), 2).unwrap();
    let elapsed = started.elapsed();
    // The plan's log assert token (QA greps `perf pixelate_4k_ms=`).
    println!("perf pixelate_4k_ms={}", elapsed.as_millis());
    assert_eq!(out.len(), 3840 * 2160 * 4);
    #[cfg(not(debug_assertions))]
    assert!(
        elapsed.as_millis() < 50,
        "4K pixelate took {elapsed:?} (gate: 50ms, release profile)"
    );
    #[cfg(debug_assertions)]
    assert!(
        elapsed.as_millis() < 2000,
        "debug-profile sanity bound; the 50ms gate runs in release"
    );
}
