# ADR-007: a CPU raster path behind the same display list

Status: accepted, implemented (flowshot-ui: `render/software`, `adapter.rs`
policy, `surface.rs` present, `handler/mod.rs` selection).

## Context

The overlay's GPU renderer pays a GPU cost twice over, and both costs are what
kill an OOM-starved desktop rather than a slow one:

- an offscreen color target at the window extent with 8x multisampling plus a
  `Stencil8` attachment — measured ~230 MB at the 4480x1440 dual-monitor union,
  which is the allocation that made the tested desktop die at the swapchain
  rather than at `request_device`;
- a Vulkan-side `encode+submit` that a software driver executes at ~63 ms/frame,
  i.e. below interactive frame rate on the very machines that have no other
  option.

Both costs are properties of the *rasterizer*, not of the content model: the
overlay's per-frame truth is a batched [`DisplayList`] of draw commands, and
nothing in the editor, the selection chrome, the pins, or the export path
requires a GPU. What requires a GPU is only the final presentation, which winit
hands us as a surface.

## Decision

Keep one display-list vocabulary and add a second rasterizer behind it:

- **`render::SoftwareRenderer`** consumes the same `DisplayList` the GPU
  `Renderer` consumes and paints into a CPU-side premultiplied, sRGB-encoded
  RGBA8 framebuffer. tiny-skia supplies the coverage-antialiased fills and
  strokes; cosmic-text shaping plus swash masks composite directly (the same
  mask source the GPU path uploads into its glyph atlas); the invert tool is a
  region filter over the same rounded-rect outline; the drop shadow is the
  shader's analytic gaussian falloff evaluated on the CPU.
- **`render::software::Presenter`** is the whole GPU side of the fallback: one
  `RGBA8Unorm` source texture plus one three-vertex fullscreen sample. The
  vertex stage emits `uv = position * 0.5 + 0.5`, which at a fragment's pixel
  center is exactly the same texel center — a 1:1 copy with no resampling and
  no frame-extent uniform.
- **The policy lives in `adapter.rs`**, next to the adapter selection, as pure
  data: `software_path_for(power, width, height)` is true for a software/virtual
  device class at any extent, or for an extent whose offscreen target
  (`pixels * (MSAA_SAMPLES + 1)`) exceeds `GPU_TARGET_MEMORY_BUDGET`. The budget
  sits between the two measured cases — under the 230 MB dual-monitor union that
  OOM'd, above the single 2560x1440 output that presented fine — so the policy
  splits observed behavior rather than guessing between it.
- **Selection is per window**, in `handler/mod.rs::init_renderers`: a window
  either owns a GPU `Renderer` or a `SoftwareRenderer` + `Presenter`, decided
  once at init with a tracing warning naming the extent and the reason.
  `WindowSurface::render_software` mirrors `WindowSurface::render`'s surface
  lifecycle (same transient-invalidation recovery, same `pre_present_notify`
  seam, same crosshair pass) and differs only in the content pass.

## What the user sees

The pill rides the SELECTION, like the quick-aids cluster and the toolbar
plate: a label-only status pill (`chrome/status`, the `StatusChip` widget)
hugs one of the four slots just outside the selection rect on a medium margin,
preferring the slot diagonally opposite the aid cluster's above-leading
default and walking through the rest, skipping any slot the toolbar plate or
the aid chips already own. It states the rasterizer that presents that
window: `GPU` quiet (dimmed contrast) when all is well, accent-tinted
`CPU fallback` when the session lost the GPU frame target and the fallback took
over. Unlike those two chrome elements, the pill never clamps into the output:
with no selection yet, or with a selection that leaves no free edge, the pill
is simply gone. The pill is decoration, not a control: the chrome funnel never
hit-tests it, so it never steals a click.

## QA override

`FLOWSHOT_RENDER_PATH` masks the policy for testing on a healthy GPU desktop:
`software` forces the CPU raster path at any extent and adapter class, `gpu`
forces the GPU path regardless of the policy. Tokens are byte-exact, the
variable is named as `adapter::RENDER_PATH_OVERRIDE_ENV` next to the policy,
and the CLI's spawned daemon inherits the environment (the same channel the
`FLOWSHOT_SHORTCUTS_PORTAL` override uses), so one shell prefix reaches every
surface. The activation is a `warn`-level tracing line naming the window, the
extent, and whether the policy or the override chose it — visible under the
CLI's default `warn` filter.

## Consequences

What the fallback buys:

- frame cost tracks ink area rather than window extent: every operation is
  bounding-box limited and the clip mask is materialized only while a clip is
  active, so a clip-free frame pays no per-pixel mask read.
- the per-frame GPU work drops to one texture upload plus one three-vertex draw,
  which a software driver can afford where the multi-pass submit could not.
- no offscreen target and no stencil attachment exist at all, so the
  allocation that OOM'd simply is not made.

What it gives up, stated exactly:

- **antialiasing is coverage-based, not multisample-based.** The framebuffer
  carries premultiplied LINEAR-light bytes and the presenter stores them into an
  sRGB target, so blending happens in the same space the GPU path blends in and
  opaque token fills agree byte for byte after that store. What remains is edge
  coverage quantization: a coverage mask steps differently from 8x MSAA, which
  the cross-rasterizer parity policy budgets (byte-exact on flat fills and image
  blits, inside the coverage-vs-MSAA budget on curved edges).
- the CPU path pays one mask composite per inked glyph rather than one atlas
  upload plus one batched draw; cost is proportional to glyph ink, which is the
  right shape for an overlay whose text is sparse.
- the fallback is a *presentation* fallback, not a Wayland-protocol fallback:
  capture, clipboard, and upload paths are unchanged and still run through the
  daemon.
