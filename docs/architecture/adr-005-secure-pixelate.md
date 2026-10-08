# ADR-005: Secure pixelate (fringe algorithm, zero interior reads)

Status: accepted, implemented (flowshot-ui editor, `pixelate.rs` +
`pixelate/noise.rs`). Live-verified on Hyprland, byte-exact through the GPU
pipeline.

## Context

The naive way to pixelate a region, average each block down and scale back
up, leaks: block means are exactly what reconstruction attacks
(Unredacter-style, dictionary-guided) consume to recover redacted text. A
redaction tool whose output is reversible is a privacy footgun with good
marketing. Flameshot knew this and ships a fringe-sampling pseudo-pixelation
for its secure mode, but it also ships an insecure mode behind a flag, and
its implementation contains a dead-weights quirk (integer-division
interpolation weights that always evaluate to 0.5).

## Decision

FlowShot's pixelate is **secure-only**; there is no insecure mode and no
config key for one (the `insecurePixelate` concept was dropped deliberately,
see [../config-reference.md](../config-reference.md#deliberate-omissions)).
The algorithm, clean-roomed from the Flameshot spec:

- **Zero interior reads.** The mosaic is derived exclusively from the four
  1-px fringe lines just outside the redacted region (falling back to the
  region's own edge line where the region touches the frame border). Nothing
  inside the selection contributes anything, so there is no block mean to
  attack.
- Grid resolution: `trunc(dim * 0.5 / max(1, size + 1))` per axis, from the
  `[tools.pixelate] size` slot. A degenerate zero grid is a no-op, not a
  crash.
- Each grid cell interpolates its four fringe samples jittered by gaussian
  sampling noise (`N(0, 5*size+1)`): one color plus eight Box-Muller
  deviates per pixel, from a pre-generated SplitMix64 (seed 42) buffer in a
  canonical order (x-then-y per fringe; fringes in top, bottom, left, right
  order; x-outer/y-inner). Deterministic per run; the C++ original's bytes
  are compiler-dependent because argument evaluation order is unspecified,
  so the clean-room pins the order explicitly.
- The noised grid is upscaled **nearest-neighbor** into the region. No
  average-downsample exists anywhere in the codebase.
- Effects are an **overlay layer**, never a frame mutation: each op bakes an
  immutable effect painted above the pristine backdrop and below the
  annotations. Undo drops the effect and the pristine frame shows through,
  lossless by construction.

## Consequences

- Interior independence is proven, not argued: the test drives three
  radically different interiors behind identical fringes and asserts
  byte-identical output. Irreversibility is quantified: a bicubic
  (Catmull-Rom) reconstruction of the mosaic grid measures below 20 dB PSNR
  against the original interior.
- Performance at 4K is bounded (scalar path ~30-36 ms release, under the
  50 ms gate), so the secure algorithm is not a perf excuse for a fast
  insecure one.
- Two seams consumers must respect: the magnifier samples the **post-effect**
  pixels (pristine plus covering effects, later wins) so the readout never
  shows redacted content through the mosaic; and a CPU export rasterizer
  must composite the effect layer explicitly (the GPU path carries it in the
  display list).
