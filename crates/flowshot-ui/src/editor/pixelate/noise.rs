//! The secure pixelate's deterministic gaussian noise (F27).
//!
//! Flameshot seeds `std::mt19937 prng(42)` and pulls from two
//! `std::normal_distribution<float>`s INSIDE the pixel loop - its own comment
//! notes the PRNG "is only used for visual effects and NOT part of the
//! security boundary" (the security comes from the interior never being
//! sampled). `std::normal_distribution` output is implementation-defined, so
//! a clean-room reimplementation pins its OWN fully-specified generator
//! instead of chasing libstdc++ bytes:
//!
//! - [`SplitMix64`] (seed [`NOISE_SEED`] = 42, the F27 constant) for the
//!   uniform stream,
//! - Box-Muller for the normal deviates (f32, the F27 precision),
//! - the whole buffer PRE-GENERATED in the canonical consumption order
//!   ("noise buffer PRE-GENERATED in canonical pixel order
//!   from the seed (SIMD consumes the SAME buffer -> byte-identity by
//!   construction)"). The canonical order is Flameshot's loop: `x` outer,
//!   `y` inner, and per output pixel one color-noise deviate followed by
//!   two sampling deviates (x then y) per fringe in [top, bottom, left,
//!   right] order - 9 deviates per pixel, [`SAMPLES_PER_PIXEL`].
//!
//! Determinism contract: identical bytes across runs of the same binary
//! (the plan's acceptance). Cross-platform bit-identity would additionally
//! pin the libm transcendentals; the acceptance criterion is per-run, and
//! the buffer layout already guarantees any future SIMD path consumes the
//! exact scalar sequence.

/// The F27 PRNG seed (`pixelatetool.cpp`: `std::mt19937 prng(42)`).
pub(in crate::editor) const NOISE_SEED: u64 = 42;

/// Normal deviates consumed per output pixel: 1 color + 4 fringes x 2 axes.
pub(in crate::editor) const SAMPLES_PER_PIXEL: usize = 9;

/// The F27 color-noise sigma (`std::normal_distribution<float> noise(0, 0.1f)`
/// - applied in the [0,1] float color space).
pub(in crate::editor) const COLOR_SIGMA: f32 = 0.1;

/// `SplitMix64`: the uniform stream behind the gaussian deviates (fully
/// specified, period 2^64, seed-42 deterministic - the visual-noise role
/// needs no cryptographic strength).
#[derive(Debug, Clone)]
pub(in crate::editor) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates the stream at `seed`.
    pub(in crate::editor) const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64-bit deviate (the canonical `SplitMix64` mixer).
    pub(in crate::editor) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Box-Muller normal deviates over [`SplitMix64`] (f32, the F27 precision).
#[derive(Debug, Clone)]
pub(in crate::editor) struct Gauss {
    rng: SplitMix64,
    spare: Option<f32>,
}

impl Gauss {
    /// Creates the sampler at `seed`.
    pub(in crate::editor) const fn new(seed: u64) -> Self {
        Self {
            rng: SplitMix64::new(seed),
            spare: None,
        }
    }

    /// The next N(0,1) deviate. `u1` is mapped into (0,1] so `ln(u1)` is
    /// finite by construction (no NaN path in the bake).
    pub(in crate::editor) fn next_f32(&mut self) -> f32 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }
        let u1 = 1.0 - unit_f32(self.rng.next_u64());
        let u2 = unit_f32(self.rng.next_u64());
        let radius = (-2.0 * u1.ln()).sqrt();
        let theta = std::f32::consts::TAU * u2;
        self.spare = Some(radius * theta.sin());
        radius * theta.cos()
    }

    /// The next N(0, `sigma`) deviate.
    pub(in crate::editor) fn scaled(&mut self, sigma: f32) -> f32 {
        self.next_f32() * sigma
    }
}

/// The high 24 bits of a u64 as a uniform f32 in [0,1) (exact: 24 bits fit
/// the f32 mantissa).
#[expect(
    clippy::cast_precision_loss,
    reason = "the 24-bit window is exactly representable in f32"
)]
fn unit_f32(bits: u64) -> f32 {
    const SCALE: f32 = 1.0 / 16_777_216.0; // 2^24
    (bits >> 40) as f32 * SCALE
}

/// The pre-generated noise buffer for one grid, in the canonical consumption
/// order (see the module header). Per pixel `p` (index `x * grid_h + y`):
/// offset `p * 9` holds the color deviate, then 4 fringe pairs
/// `(x-deviate, y-deviate)` for [top, bottom, left, right].
#[derive(Debug, Clone, PartialEq)]
pub(in crate::editor) struct NoiseBuffer {
    data: Vec<f32>,
    grid_h: u32,
}

impl NoiseBuffer {
    /// Generates the buffer for a `grid_w x grid_h` grid at tool `size`
    /// (the F27 sampling sigma = `5 * size + 1`).
    pub(in crate::editor) fn generate(grid_w: u32, grid_h: u32, size: u32) -> Self {
        let sampling_sigma = sampling_sigma(size);
        let mut gauss = Gauss::new(NOISE_SEED);
        let pixels = grid_w as usize * grid_h as usize;
        let mut data = Vec::with_capacity(pixels.saturating_mul(SAMPLES_PER_PIXEL));
        for _x in 0..grid_w {
            for _y in 0..grid_h {
                data.push(gauss.scaled(COLOR_SIGMA));
                for _fringe in 0..4 {
                    data.push(gauss.scaled(sampling_sigma));
                    data.push(gauss.scaled(sampling_sigma));
                }
            }
        }
        Self { data, grid_h }
    }

    /// The color deviate for output pixel `(x, y)`.
    pub(in crate::editor) fn color(&self, x: u32, y: u32) -> f32 {
        self.data[self.base(x, y)]
    }

    /// The sampling deviate pair `(x, y)` for output pixel `(x, y)` and
    /// fringe `fringe` (0=top, 1=bottom, 2=left, 3=right).
    pub(in crate::editor) fn sample(&self, x: u32, y: u32, fringe: usize) -> (f32, f32) {
        let at = self.base(x, y) + 1 + fringe * 2;
        (self.data[at], self.data[at + 1])
    }

    fn base(&self, x: u32, y: u32) -> usize {
        (x as usize * self.grid_h as usize + y as usize) * SAMPLES_PER_PIXEL
    }
}

/// The F27 sampling-noise sigma: `5 * size + 1` (saturating; the dispatch
/// clamps size to [1,50] but the bake API accepts any u32).
#[expect(
    clippy::cast_precision_loss,
    reason = "sigma magnitudes far below 2^24 are exact; saturating caps at u32::MAX"
)]
pub(in crate::editor) fn sampling_sigma(size: u32) -> f32 {
    size.saturating_mul(5).saturating_add(1) as f32
}
