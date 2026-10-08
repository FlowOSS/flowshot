# crates/flowshot-ui/src

220 files, 16 module dirs (29 nested), 87 re-exports via `lib.rs`.
The crate root covers packaging; this covers what breaks silently if guessed.

## OVERVIEW

Two layers, split so the interesting logic never touches a GPU or a display: a
**headless core** (`state.rs` + `router.rs`) and a **window shell**
(`runtime.rs`/`app.rs`/`handler/`/`surface.rs`/`gpu/`).

## STRUCTURE

| Layer | Modules | Owns |
|-------|---------|------|
| Headless core | `state`, `state/route`, `router`, `input`, `launch` | `OverlayCore`, coordinate mapping, the route funnel, preselect math |
| Window shell | `runtime`, `app`, `handler/`, `surface`, `gpu/`, `monitor`, `adapter`, `crosshair` | winit loop, per-monitor windows, device/surface policy |
| Renderer | `render/` (20 files) | `Renderer`, `DisplayList`, `Command`, `Color`, `TextureStore` |
| Frame build | `frame`, `backdrop/`, `export`, `completion` | the ONE paint path + frozen-frame pixel prep |
| Behavior | `selection/`, `editor/`, `editor/tools/`, `chrome/`, `pins/` | the F27 selection spec, tool framework, chrome, pin windows |
| Design system | `widgets/`, `motion/`, `egui_host/` | token-driven primitives, easing, the embedded egui stack |
| egui windows | `settings/`, `launcher/`, `consent/` | the only three egui surfaces |

## COORDINATE SPACES (the top bug source)

winit delivers **surface-local physical px**. `InputRouter::to_global` maps
`(WindowSlot, local physical)` → **global logical** via
`flowshot_core::geometry::OutputLayout`. The mapping is linear and *unclamped*
on purpose: mid-drag the compositor's implicit pointer grab keeps sending
motion to the origin window while the cursor is logically over another
monitor. Clamping is explicit (`to_global_clamped` / `clamp_point`).

- Selection rect: **global logical** — one rect spans every monitor.
- `render::Command` / `DisplayList`: **physical px**, per window.
- Magnifier: geometry is window-local physical; its sample arrives frame-physical.
- Export: per-output physical crops via `completion::composite_selection` → `OutputLayout::crop_rects` (#4871).

## EVENT + PAINT CONTRACTS

- Routing order: **chrome first** (widget parity), then the F27 chain —
  picker > right-click > active tool > edit commit > object select >
  selection engine. `state/route.rs` + `state/route/pointer.rs` are the funnel.
- The six-stage Esc cascade is owned by `selection/cascade.rs` (the final
  contract); the funnel only applies the popped step's editor-side reaction.
- `frame::build_overlay_frame` is the single display-list path shared by the
  live shell and the offscreen QA harnesses: backdrop → grid → editor →
  selection chrome → editor chrome → magnifier, at a caller-supplied `now` so
  motion stills are deterministic under a synthetic clock.

## CONVENTIONS

- **250 pure-LOC ceiling** → split into `foo.rs` + `foo/<half>.rs` and name the
  precedent in the doc comment (`editor/events.rs` + `events/pointer.rs`,
  `chrome/side_panel.rs` + `side_panel/paint.rs`, `state/route.rs` + `route/pointer.rs`).
- Both module-root styles coexist: `src/<name>.rs` + `src/<name>/` (backdrop,
  editor, launch, pins, selection, state — 6) and `src/<name>/mod.rs` (chrome,
  consent, egui_host, gpu, handler, launcher, motion, render, settings,
  widgets — 10). Match the neighbor; neither style is preferred.
- Tests live in a sibling file: `#[cfg(test)] mod tests;` → `tests.rs`, or
  `*_tests.rs` when a module has several (`blur_tests`, `pixelate_tests`,
  `wiring_tests`, `text_tests`). Inline `mod tests {}` only for tiny modules.
- Token-driven everywhere: `widgets/` derives every color/radius/spacing from
  `flowshot_core::tokens::DesignTokens`; `motion/` evaluates bezier curves from
  tokens; `egui_host/theme/projection.rs` projects tokens into `egui::Style`.
- Color path: token hex → premultiplied **linear** light vertices → sRGB
  targets, so opaque flat fills reproduce token bytes exactly.

## ANTI-PATTERNS (THIS DIRECTORY)

- **egui is a guest, never a second windowing stack (MUST-NOT).** The overlay
  and editor never touch egui; `egui_host::input` hand-feeds it from raw winit events.
- No CLI parsing in `launch/` — the `--region` grammar is flowshot-cli's.
- `unwrap`/`expect`/`panic!` are test-only here; the renderer never panics
  (degenerate geometry is skipped with a tracing log, a missing texture draws
  the magenta placeholder).
- Do not add a frame loop: frames fire only on `RedrawRequested`, and
  `handler/` idles in `about_to_wait` so an idle overlay costs zero CPU.
- Never rely on the compositor cursor — it is hidden and `crosshair.rs` draws
  its own (the #1659-class invisibility fix).
