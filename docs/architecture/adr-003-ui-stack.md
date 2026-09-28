# ADR-003: UI stack: winit + wgpu + cosmic-text (one recorded exception)

Status: accepted, implemented (flowshot-ui).

## Context

The overlay has unusual requirements for a Rust UI: one borderless
fullscreen window per output, a frozen multi-monitor backdrop presented
pixel-exactly (see [ADR-001](adr-001-physical-first-geometry.md)), GPU
rendering fast enough for a 4K frame budget, real IME for the text tool
(CJK input included), and zero X11 fallback. A general-purpose retained UI
toolkit (GTK, Qt, egui for the whole surface) either cannot express the
overlay window model or pulls in dependencies the project rejected.

## Decision

The UI stack is:

- **winit 0.30** for windowing and input (Windows are created in `resumed()`,
  per the 0.30 lifecycle; IME is always-on from spawn, which is what makes
  the text tool's CJK path work).
- **wgpu 0.20** for rendering, restricted to `Backends::PRIMARY` (Vulkan on
  Linux). `Backends::all()` eagerly initializes the GLES/EGL backend even
  when a Vulkan adapter is selected, and NVIDIA's EGL-Wayland shim segfaults
  in `eglTerminate` at instance drop (core-dump proven during live QA). The
  trade-off is recorded: a GL-only environment gets a typed no-adapter error
  instead of a teardown crash.
- **cosmic-text 0.19** as the single text stack (shaping plus its
  SwashCache rasterizer). No glyphon: no glyphon release pairs with the
  wgpu 0.20 pin (verified against crates.io).
- **lyon** for tessellation, **resvg** (build-time only) for rasterizing the
  vendored SVG icon set into the texture atlas.
- A 4096-px minimum texture-dimension floor is enforced at adapter selection
  (never `downlevel_defaults`, whose 2048 cap is below real monitor widths),
  and surface extents are validated against the device limit before every
  configure, because wgpu panics on oversized configs.

### The one exception: egui for settings

The settings window (a normal decorated window, not the overlay) uses
**egui 0.28 embedded in our wgpu renderer via egui-wgpu**, confined to that
surface. The overlay and editor remain winit+wgpu+cosmic-text only.

The recorded version dance: egui-winit 0.28.1 requires winit 0.29 while the
workspace pins winit 0.30, and no egui release pairs (wgpu 0.20, winit 0.30).
So **egui-winit is deliberately not used**; the settings surface feeds egui
input manually from FlowShot's own winit 0.30 input router
(`settings::input`). If a future egui release pairs both pins, the workspace
bumps together or stays on the manual feed; the comment block in
`crates/flowshot-ui/Cargo.toml` carries the constraint at the point of use.

## Consequences

- There is exactly one windowing stack and one text stack; new UI surfaces
  do not get to choose a different one. The purity gate
  ([ADR-006](adr-006-cross-platform-gates.md)) keeps even the Wayland
  `app_id` setter out of the library crate (it is a platform extension); the
  binary layer applies it through an injection seam.
- The stack is headless-testable: input routing, the selection engine, the
  editor, chrome, and pins run against synthetic events with no event loop,
  which is how most of the 1200-test suite exists.
- Idle CPU is near zero by construction (event-driven `WaitUntil` scheduling,
  measured 0.17% over an idle overlay during live QA).
- Known cost: winit exposes no portable app-id setter and no pinch gestures
  on Wayland touchpads (two-finger touch pinch is implemented instead), and
  Wayland gives no initial-position API for toplevels. All three are
  documented behavior, not bugs to chase.
