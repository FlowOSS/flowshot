# ADR-001: Physical-first geometry

Status: accepted, implemented (flowshot-core `geometry` module).

## Context

Flameshot's mixed-DPI bugs (upstream issue #4871 is the reference case) come
from one root cause: the code mixes logical and physical pixels in the same
variables, converting at ad-hoc points, sometimes twice, sometimes never. On
a scale-2 output a region capture comes out doubled, halved, or offset, and
the failure mode shifts with every refactor because nothing in the type
system says which space a number lives in.

FlowShot runs on exactly the setups where this breaks: mixed-scale multi-
monitor Wayland sessions.

## Decision

All geometry inside FlowShot is **physical pixels first**. Scale factors
convert at exactly one boundary, and the type system refuses to mix spaces:

- Coordinates are newtyped: `PhysicalPx` and `Logical` are different types,
  and `Point`/`Size`/`Rect` are generic over the coordinate space.
- `ToLogical` is implemented only for physical types; `ToPhysical` only for
  logical types. Double scaling is a compile error, not a code-review item.
- Each direction has exactly one conversion function. Conversions are total:
  an invalid scale (NaN, zero, infinite) falls back to 1.0 rather than
  panicking in library code.
- Region cropping intersects in logical space per output, then converts
  **edges** (not widths) with *that output's* scale into its post-transform
  buffer. Averaging scales across outputs is the specific #4871 bug and is
  regression-tested against (`mixed_scales_crop_per_output_not_averaged`).
- Rects are half-open `[x, x+w)` so adjacent outputs tile deterministically.
- The geometry HUD reports global logical pixels (the hyprctl/layout-algebra
  space); a spanning mixed-DPI rect has no single physical answer.

Frame delivery from compositors differs in orientation, and the rule absorbs
that at one seam: frames carry native pre-transform pixels plus a `transform`
tag. ICC delivers post-transform buffers (Hyprland always reports transform
normal; wlroots sends post-transform sizes) and gets inverse-remapped;
wlr-screencopy delivers native buffers and needs no remap. A hard dimension
guard (`BufferSizeMismatch`) rejects a frame whose size contradicts the
output's expected buffer size.

## Consequences

- A capture of a scale-2 monitor is exactly its physical size: never doubled,
  never halved. Verified live on a 3840x2160-at-scale-2 headless output and
  on the mixed-scale QA matrix.
- Every consumer downstream (overlay, magnifier, pins, export) reasons in one
  space and converts once. The magnifier is physical-first at any scale; pins
  convert DPR exactly once for window dressing and never divide by it.
- The cost is discipline: new code cannot lazily pass `f64` around. This is
  the point.
- One open verification item: Hyprland computes cursor-session positions
  logical-relative, the protocol specifies physical; the two coincide at
  scale 1 (live-verified) and the scale-2 divergence check is queued in the
  GUI QA batch (see [../verification.md](../verification.md)).
