# Learnings — flowshot

Conventions, patterns, and successful approaches discovered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## Todo 2: Design tokens + TOML config + versioned migration (flowshot-core)

**What landed**
- `tokens.rs`: `DesignTokens { palette, spacing, radii, shadows, typography, easings }`, all serde + `Default`. Accent `#2AA198` (provisional), contrast `#1A1A2E`, dim 190. 4 easing curves (standard/decelerate/accelerate/sharp) exposed via `Easing::standard()` etc. + `DesignTokens::easing(name)` lookup.
- `config.rs`: grouped schema `[capture] [save] [editor] [tools.*] [pin] [upload] [ui] [daemon]` with `CONFIG_VERSION = 2`, migration registry `MIGRATIONS: &[(from_version, fn)]`, `ConfigError` (thiserror), `load/save/from_toml_str/to_toml_string`, `FromStr` impl.
- `lib.rs`: exposes `config`/`tokens` modules + re-exports `Config`, `ConfigError`, `CONFIG_VERSION`, `DesignTokens`.

**Key patterns that worked**
- Byte-stable TOML: scalar `config_version` declared FIRST in `Config` (toml crate errors on values-after-tables); container-level `#[serde(default)]` on every group so partial configs fill defaults; `Option<Region>` with `skip_serializing_if = "Option::is_none"` keeps `last_region` out of output until first capture.
- Migration driver: parse to `toml::Value`, missing `config_version` => 0 (legacy), reject `> CONFIG_VERSION` with `FutureVersion`, loop registry steps rewriting the table, stamp version, then `Value::try_into::<Config>()`. v1->v2 renames `save.filename_template` -> `filename_pattern` without clobbering an explicit new key.
- UiConfig defaults are seeded from `tokens::Palette`/`Typography` — single source of truth between tokens and config.
- No-unwrap tests: `#[test] fn x() -> Result<(), ConfigError>` for happy paths, `assert!(matches!(..))` for error paths (clippy `unwrap_used`/`expect_used` are workspace-deny and apply to test targets too).

**Gotchas hit**
- `text.contains("last_region")` false-positives on `save_last_region` — assert on parsed `toml::Table::contains_key` instead of substrings.
- Edition 2024 let-chains required by `clippy::collapsible_if` (deny via `clippy::all`): collapse `if let && let && cond` instead of nesting.
- `clippy::derivable_impls` (deny) flags manual `Default` impls equal to derive — use `#[derive(Default)]` for `DaemonConfig`/`ToolsConfig`/`ArrowToolConfig` + `#[default]` enum variants; manual impls only where values differ from type defaults.
- `clippy::unnecessary_wraps` (pedantic) fires on always-`Ok` migration fns; kept fallible signature for registry uniformity with `#[allow]` + justification.
- `struct_excessive_bools` (pedantic) on `EditorConfig` — allowed with reason: flags are spec-mandated independent settings.

**Verification**: `cargo test -p flowshot-core` 18 unit + 1 doctest green (13 under `config::`), clippy `--all-targets` 0 warnings, `cargo fmt --check` clean, default-config write->reload byte-stable (string + file level tests).

## Todo 2: scene graph + snapshot undo + counter renumbering (flowshot-core::scene)
- `Box<dyn ToolObject>` clone pattern: supertrait `ToolObjectClone` with blanket `impl<T: ToolObject + Clone + 'static>` — concrete types only derive Clone.
- Deref coercion `&Box<dyn T> -> &dyn T` does NOT auto-apply in return position; use `let object = self.objects.get(id)?; Some(&**object)`. Explicit `&Box<...>` type annotation triggers clippy::borrowed_box (deny via clippy::all); closure `.map(|o| &mut **o)` hits a WF lifetime error (`'1 must outlive 'static`) — the let-? + `&mut **object` form compiles clean.
- `f32: From<u32>` does not exist (precision loss); use `chars as f32` + `#[allow(clippy::cast_precision_loss)]`.
- Edition 2024 let-chains required by clippy::collapsible_if (deny): `if let Some(x) = opt && cond { }`.
- UndoStack: full (before, after) snapshot pairs + cursor; undo/redo return Option<Scene> clones, None = silent no-op; push truncates redo tail then evicts oldest while over limit; limit=0 disables history.
- Counter numbering lives ONLY in Scene: add_object auto-numbers count==0 with max+1; remove_object decrements counts > removed; restore_object force-assigns max+1 (undo-restore rule). Snapshot undo restores exact pre-op numbering; max+1 applies to re-add flows.
- Scene ids are dense arena indices; remove compacts + remaps z_order (invariant: z_order is a permutation of 0..len — proptest-verified over random add/remove/raise/lower sequences).
- serde: `#[serde(tag = "type")]` enum ToolObjectData bridges dyn objects; tags match type_id() strings; Scene Serialize/Deserialize delegate via SceneData; toml crate roundtrips internally-tagged enums fine (test uses toml, no serde_json dep needed).
- config bridge: `UndoStack::from_undo_limit(EditorConfig::undo_limit)`; DEFAULT_UNDO_LIMIT=100 matches config default (test asserts the tie).
- proptest is in flowshot-core [dependencies] (not dev-deps) — usable from #[cfg(test)] directly; import `proptest::prop_assert_eq` explicitly inside test mod.

## Todo 2: geometry (flowshot-core::geometry) — 2026-09-25
- **Space-separated newtypes**: `PhysicalPx(i32)` / `Logical(f64)`; `Point<C>`/`Size<C>`/`Rect<C>` generic over coord with aliases (`PhysicalRect`, `LogicalRect`, ...). `ToLogical` implemented ONLY for physical types, `ToPhysical` ONLY for logical → double scaling is a compile error. Exactly one conversion fn per direction (trait method), also for compound types.
- **Conversions are total**: invalid scale (NaN/≤0/inf) falls back to 1.0 in `to_logical`/`to_physical` (no panics in lib); validated construction via `OutputInfo::new` → `Result<_, GeometryError>` (thiserror). `to_physical` rounds half-away-from-zero, saturates at i32 bounds.
- **Physical-first crops**: `OutputLayout::crop_rects(region)` intersects in logical space per output, then converts EDGES (not widths) with THAT output's scale into its post-transform buffer (`buffer_size()` = transform.apply_to_size(physical_size)). Averaged-scale bug is regression-tested (`mixed_scales_crop_per_output_not_averaged`).
- **Transform**: 8 wl_output-convention variants (flip first, then rotate CCW). `map_point` (src→dst, usize, saturating), `remap_buffer<T: Copy>` (channels-generic, Result on size errors), `inverse()`, `TryFrom<u32>` wire values 0..=7. Flips are involutions; t⁴ = identity for all.
- **f64 gotcha (important for future geometry work)**: `x + (right - x) != right` in f64 (1 ulp). Exact-equality proptests for rect algebra (self-intersection, union absorption, clamp idempotency) MUST use integral-valued f64 grids; fractional layouts need ~1e-9 tolerance. Layouts anchored at x=0,y=0 keep union bounds exact (adding 0.0 is exact).
- **Rects are half-open** [x, x+w) — boundary points belong to the output starting there; makes `output_at` deterministic for adjacent outputs.
- **proptest in nested `prop_flat_map`**: closures are `Fn`, can't move captured values — clone per nesting level (`let layout_x = layout.clone(); move |x| ...`).
- CI gate is `cargo clippy -- -D warnings` with workspace pedantic lints → lib needs `#[must_use]` on pure fns (incl. trait methods returning Self), `# Errors` doc sections, no bare `as` casts (use `f64::from`, `usize::try_from`, or one allowed helper `round_to_i32`).
- proptest moved to `[dev-dependencies]` in flowshot-core (workspace lists it under Testing); lib stays pure: std + serde + thiserror only.
- Result: 41 geometry tests green (28 proptest fns × 256 cases + 13 unit), clippy --all-targets -D warnings clean, fmt clean.

## Todo 5: capture backend trait + negotiation ladder + mock (flowshot-capture) — 2026-09-25
- **Crate shape**: `backend.rs` (CaptureBackend trait + CaptureOpts + PermissionResult), `kind.rs` (BackendKind), `frame.rs` (Frame/FrameBuffer/FrameFormat/OutputRef), `cursor.rs` (CursorEvent/CursorStream = `Pin<Box<dyn Stream<Item=CursorEvent> + Send>>`), `negotiate.rs` (DesktopEnv/CapabilityProbe/NEGOTIATION_LADDER/negotiate), `error.rs` (CaptureError), `mock.rs` + `mock/pixels.rs` (MockBackend; split at ~250 pure-LOC: contract vs pixel plumbing).
- **async-trait 0.1.92** added to root [workspace.dependencies] (was already in Cargo.lock as transitive dep - zero lockfile churn). tokio moved to flowshot-capture [dev-dependencies] (lib is runtime-agnostic; tests use #[tokio::test] to prove tokio compat of the Send-bounded futures).
- **BackendKind declaration order = ladder order** (ICC, Screencopy, Kwin, PortalCast, PortalShot, then roadmap X11/Windows/MacOs) so Ord-derived BTreeSet iteration matches the ladder; negotiate() still iterates the explicit NEGOTIATION_LADDER const (robust to reordering). Roadmap kinds: documented unconditional variants (NOT cfg-gated - cfg arms in every downstream match was uglier); `is_roadmap()` + `CapabilityProbe::supports()` hard-false them, so they're non-constructible via negotiation AND via force_backend.
- **force_backend semantics decided**: override wins over ladder ORDER but not over CAPABILITIES - forced kind unsupported by probe => `NoBackendAvailable { missing: vec![forced] }` failing fast at negotiation instead of dying at protocol bind. (Rejected alternative: unconditional override as probe-bug escape hatch - can add an ignore_probe flag later if a probe ever misfires.)
- **NoBackendAvailable message**: thiserror `#[error("...{}", ProtocolList(missing))]` with a private Display wrapper joining `BackendKind::protocol_name()` (wire names: ext-image-copy-capture-v1, zwlr-screencopy-v1, org.kde.KWin.ScreenShot2, org.freedesktop.portal.ScreenCast, org.freedesktop.portal.Screenshot). Tests assert every name appears in the string.
- **MockBackend**: kind is configurable (`new(kind)`) since BackendKind has no Mock variant by design; 2 embedded 8x6 PNG fixtures (solid red/green, generated with python stdlib zlib - no PIL needed: IHDR color-type-2 + zlib IDAT + CRC32); fixture cycled by output index + nearest-neighbour resampled to physical_size, so with_outputs() accepts arbitrary layouts. Region stitch: clamp_region_to_layout -> per-output intersection+physical_crop (core algebra) -> oriented() remap via Transform::remap_buffer for rotated outputs -> blit at scale 1.0. paint_cursor paints a white 3x3 glyph at (4,4) => observable buffer diff in tests.
- **Frame contract**: buffer = native pre-transform orientation, `transform` metadata tells consumers to remap (matches todo 15's upload path); region frames = Composite OutputRef, scale 1.0, Transform::Normal. FrameBuffer.data = BytesMut (plan said "BytesMut-or-vec"; Bytes rejected: downstream takes frames by move, refcount clone unneeded).
- **Gotchas**: `Result::filter` doesn't exist (use `.ok().filter()`); clippy::needless_pass_by_value fires on by-value view structs passed to blit helpers (pass &/&mut or derive Copy); `#[expect(clippy::too_many_arguments)]` is UNFULFILLED at exactly 7 args (clippy threshold = warn above 7) => unfulfilled_lint_expectations fails -D builds - group params into a domain struct (DestinationRect) instead; doc_markdown (pedantic) flags KWin/macOS/PipeWire/FlowShot/ScreenCaptureKit AND dashed D-Bus in doc comments - backtick them all; doctests calling trait methods need the trait imported (`use flowshot_capture::CaptureBackend`) even when the concrete type is in scope.
- **Concurrent-worker reality**: workspace-wide gates (build/clippy/fmt --workspace) can be blocked mid-flight by another worker's broken crate manifest/source (flowshot-ui was mid-edit: `mod ui` without ui.rs broke even `cargo fmt --check` workspace-wide mod resolution). Fallback that keeps verification honest: `rustfmt --check --edition 2024 <files>` directly (no cargo/workspace needed) + `-p` scoped test/clippy/build + build every crate NOT depending on the broken one. Record the blocked state + exact foreign errors in evidence, never claim workspace-green that wasn't observed.
- Result: 31 unit tests (15 negotiate matrix + 12 mock + 2 error + 2 kind) + 15 doctests green; clippy -p --all-targets -D warnings clean; purity greps (wayland/winit) empty; core 81 tests still green; capture-wayland + actions compile against the new contract.

## Todo 13: winit+wgpu multi-monitor overlay runtime (flowshot-ui) — 2026-09-25

**What landed**
- Headless/shell split: `router.rs` (InputRouter: WindowSlot + surface-local physical f64 -> global logical via OutputLayout; unclamped linear mapping = implicit-grab spanning enabler; explicit clamp_point/to_local inverse) + `state.rs` (OverlayCore: cursor track, IME status, exit flag, `route()` funnel) are pure data — no winit handles, no GPU; `handler.rs`/`app.rs`/`runtime.rs` are the winit+wgpu shell. Test seam `inject_event(SyntheticInput)` on OverlayCore `#[cfg(any(test, feature = "test-drive"))]` (so plain `cargo test -p flowshot-ui` exercises it) + `OverlayHandle::inject_event` for the live loop via EventLoopProxy.
- 29 unit + 2 doctests green headlessly; clippy clean in 3 configs (default, --all-targets, +test-drive, bare-with-doctests).

**winit 0.30.13 API facts (verified against vendored source, not memory)**
- `EventLoop::create_window` is DEPRECATED; windows must be created from `ActiveEventLoop` inside `resumed()`. `available_monitors()`/`primary_monitor()` exist ONLY on ActiveEventLoop (returns `impl Iterator`, not Vec). `ActiveEventLoop::exit()` is the clean all-windows teardown; run_app returns Ok.
- `ApplicationHandler<T>` callbacks take `&ActiveEventLoop` (concrete struct in 0.30.5+, not the old generic target). `EventLoopProxy::send_event` (NOT send_user_event).
- `WindowAttributes`: no portable app-name setter. app_id on Wayland comes ONLY from `WindowAttributesExtWayland::with_name(general, instance)` (platform ext) — PURITY GATE blocks it in flowshot-ui, so app_id stays unset in the example; the real binary (todo 35, flowshot-cli NOT purity-gated per todo 42) must set it. QA-batch caveat recorded in gui-qa-batch.md.
- `MonitorHandle::position()` on Wayland = LOGICAL compositor position * scale (physical px) — divide back via `PhysicalPoint::to_logical(scale)` to get global logical origin. `size()` = pre-transform mode size; transform NOT exposed → assume Transform::Normal and refine the bound OutputInfo from the window's first `Resized` (real post-transform surface extent) via `InputRouter::update_surface_size`. Todo 15's capture-provided layout supersedes all of this.
- `WindowEvent::KeyboardInput { device_id, event, is_synthetic }` — NO `repeat` field on the variant; `repeat` lives on `KeyEvent`. `set_ime_allowed(true)` exists and is NOT deprecated in 0.30.13 (deprecation is 0.31) — calling it once at spawn IS the always-on model (D7). `Ime` has exactly 4 variants in 0.30.13 (Enabled/Preedit(String, Option<(usize,usize)>)/Commit/Disabled — no DeleteSurrounding) and is Clone-not-Copy (String payload) → SyntheticInput/InputEvent derive Clone only.
- `create_window` returns `Window` by value (0.30); wrap in `Arc::new` yourself for wgpu.

**wgpu 0.20.1 API facts**
- `RenderPipelineDescriptor` has NO `cache` field (that's 22+). `compilation_options: PipelineCompilationOptions::default()` (clippy rejects bare `Default::default()` there). Inline WGSL = `ShaderSource::Wgsl(Cow::Borrowed(..))`; `include_wgsl!` is file-only.
- `create_surface(Arc<Window>)` works via blanket `WindowHandle for T: HasWindowHandle+HasDisplayHandle+Send+Sync` → `Surface<'static>` (surface keeps its own Arc to the window). `Instance` need NOT be retained: adapter/device/surface each hold `Arc` to the shared context.
- `request_adapter`/`request_device` are async → `futures::executor::block_on` once at startup (futures added to flowshot-ui deps; workspace inheritance FORBIDS `default-features = false` overrides in member Cargo.toml — hard cargo error).
- `SurfaceConfiguration` = wgpu-types generic aliased to `Vec<TextureFormat>` view_formats; has `desired_maximum_frame_latency`. PresentMode::Fifo always supported. get_current_texture: Outdated|Lost → reconfigure+skip frame, Timeout → skip, OutOfMemory → typed error.

**Dep hygiene**
- Pruned egui/egui-winit/egui-wgpu/cosmic-text/glyphon/lyon/tokio from flowshot-ui (unused in todo 13): egui-winit 0.28 drags winit 0.29 and egui-wgpu drags wgpu 22 → lockfile had TWO winit + TWO wgpu majors (deny.toml bans version drift). After prune: single winit 0.30.13 + wgpu 0.20.1. Todo 14 re-adds cosmic-text/glyphon/lyon; todo 36 MUST version-match egui to the winit 0.30 pin (plan dep guardrail).
- `cargo test` BUILDS examples but never RUNS them — safe under the no-live-GUI amendment.

**Clippy/lint gotchas (workspace pedantic + deny)**
- Bare `cargo clippy` (CI form, no --all-targets) ALSO checks DOCTESTS → `expect_used`/`unwrap_used` deny fires inside doc examples; write doctests with `?` + `# fn main() -> Result<...>` wrappers.
- `doc_markdown` flags bare "FlowShot" (CamelCase) in doc comments → backtick it (flowshot-core convention).
- `needless_pass_by_value` fires on by-value structs whose fields are only borrowed (SurfaceSpec) AND on the seam `inject_event(SyntheticInput)` — the latter keeps by-value for cross-thread send symmetry with `#[allow]` + justification.
- `float_cmp` in tests: allowed at test-mod level (crate convention, fixtures integral-valued per geometry learnings); `f32::from(0x2A)` fails (hex literals default i32, `f32: From<i32>` missing — the geometry-notepad gotcha again; suffix `0x2Au8`).
- `#![forbid(unsafe_code)]` blocks the usual bytemuck-style vertex cast → safe `to_le_bytes` Vec (96 B/frame, only on motion).

**Design decisions recorded**
- Router keys are our own `WindowSlot(usize)` (dense, monitor-order), NOT winit WindowId (opaque, not headless-constructible) — shell keeps HashMap<WindowId, WindowSlot>. This is what makes the mapping math unit-testable without an event loop.
- WAYLAND_DISPLAY precheck is a portable `std::env::var_os` probe (uppercase — invisible to the lowercase purity grep by construction) implementing the acceptance failure path without platform imports; X11 roadmap phase revisits it.
- Crosshair draws on the window that LAST RECEIVED MOTION (during implicit grab the neighbor window gets no events — per-surface clipping is the Wayland-honest behavior; spanning selection DATA lives in global logical space regardless, todo 16).
- OverlayRuntime::run() destructures self (partial move, no Drop impl) — EventLoop is consumed by run_app; init failure inside resumed() is stored and re-surfaced as Err after the loop exits (typed error → example exit 1).

## Todo 6: Wayland connection + registry probe + output enumeration (flowshot-capture-wayland) — 2026-09-25

**What landed**
- `CaptureThread`: dedicated `flowshot-capture` thread owning its own `wayland-client` Connection + calloop 0.13 EventLoop (`calloop-wayland-source` 0.3.0 `WaylandSource`). Foreign threads talk via bounded calloop `sync_channel(8)` commands + std mpsc replies, 10s deadlines everywhere (startup rendezvous, snapshot replies). Drop/shutdown = `LoopSignal::stop()+wakeup()` -> join.
- Modules: `error.rs` (ConnectError with env-derived hint + ProbeError), `desktop.rs` (pure `desktop_env(xdg, hyprland_sig)` sniff), `globals.rs` (`Global` + `ProtocolGlobals::from_globals` string-interface detection + `to_capability_probe`), `output.rs` (`OutputData` plain-event-data -> `OutputInfo`), `transform.rs` (all-8 wl_output->geometry map), `session.rs` (`CaptureState` + 3-roundtrip `collect_session` + `SessionSnapshot`), `dispatch.rs` (4 hand-rolled `Dispatch` impls), `thread.rs`, `examples/probe.rs`.
- LIVE-verified on Hyprland: probe JSON == `hyprctl monitors -j` (jq diff EMPTY incl. transform->wire-value mapping); `WAYLAND_DISPLAY=nonexistent` -> typed hint on stderr, exit 1, no panic.

**wayland-rs API facts (verified against vendored 0.31.15/0.32.13 sources, not memory)**
- wayland-protocols 0.32 REORGANIZED modules: xdg-output is `xdg::xdg_output::zv1::client` (NOT `unstable::xdg_output::v1` as in 0.31), gated behind the `unstable` cargo feature; ext-image-copy-capture + ext-image-capture-source (both source managers live in ONE xml) are `ext::image_copy_capture` / `ext::image_capture_source` behind `staging`. New scanner output nests TYPES in per-interface modules: `...::zv1::client::zxdg_output_v1::ZxdgOutputV1` (sctk 0.19.2 src/output.rs is the ground-truth reference).
- Event enum fields for protocol enums/bitflags arrive wrapped in `WEnum<T>`: `transform.into_result() -> Result<T, WEnumError>`, `flags.into_result().is_ok_and(|f| f.contains(Mode::Current))`. Generated event enums AND value enums are `#[non_exhaustive]` -> every match needs a `_` arm (clippy `match_same_arms` then fires on `Done => {}` + `_ => {}` -> merge into `Done | _ => {}`).
- `conn.display().get_registry(&qh, udata)` takes USER DATA (2 args) - `Dispatch<WlRegistry, ()>` is a choice, not fixed. `WlRegistry::bind` infers from annotation: `let o: WlOutput = registry.bind(name, ver, qh, key);`.
- `Connection::connect_to_env()` has NO wayland-0 fallback: WAYLAND_DISPLAY unset -> `ConnectError::NoCompositor` immediately; variants are only NoWaylandLib/NoCompositor/InvalidFd (defined in wayland-client conn.rs, not backend).
- calloop 0.13: `EventLoop<'l, Data>` is LIFETIME-PARAMETERIZED (return `EventLoop<'static, _>` from builders); loop stop = `get_signal()` -> `LoopSignal::stop()+wakeup()` consumed by `run()` (NOT `set_exiting`, that's gone); `insert_source` callback is `(Event, &mut Metadata, &mut Data)` - Channel's Metadata destructures as `()`; `InsertError { error, inserted }` - map `.error` into typed errors.
- Interface wire names for detection (from cached protocol XMLs): `ext_image_copy_capture_manager_v1`, `ext_output_image_capture_source_manager_v1`, `ext_foreign_toplevel_image_capture_source_manager_v1`, `zwlr_screencopy_manager_v1`, `zxdg_output_manager_v1`, `wl_shm`, `zwp_linux_dmabuf_v1` (stable xml KEEPS the zwp_ wire name), fractional-scale is `wp_fractional_scale_manager_v1` in 0.32 staging but OLD compositors advertise `zwp_` - detect BOTH spellings.

**Dependency alignment (root Cargo.toml, recorded for dep-pin rule)**
- wayland-protocols 0.31.1 -> { 0.32.13, features=["client","unstable"] }; wayland-protocols-wlr 0.2.0 -> 0.3.12; + calloop-wayland-source 0.3.0; + serde_json 1.0.151 (example); wl-clipboard-rs 0.8.0 -> 0.9.3.
- wl-clipboard-rs was the LAST holder of wayland-protocols 0.31.2/wlr 0.2.0/nix 0.28 duplicates; 0.9.3 rides 0.32/0.3/rustix-1 (all already in lock via winit->sctk 0.19.2 / others) and flowshot-actions has ZERO code usage of it yet (todos 28-31) -> bump eliminated 3 duplicate groups with zero code churn. New transitive crates all MIT (deny allow-list ok). sctk 0.21 deliberately NOT added: lock carries sctk 0.19.2 via winit -> a second sctk would be NEW drift; hand-rolled binds (libwayshot pattern, draft F6) chosen.
- Remaining `cargo tree -d` groups (base64/rand/syn/thiserror/zbus families etc.) are pre-existing transitive drift from wgpu/reqwest/proptest/zbus stacks - untouched, not wayland-related.

**Gotchas**
- jq 1.8 PRESERVES float literal rendering: `1.0` stays `1.0`, so probe-vs-hyprctl diffs need `def norm: if . == floor then floor else . end` on both sides (still exact arithmetic, tolerance zero). hyprctl reports transform as wire INT (0), we serialize enum name -> map {Normal:0,...} in the jq filter.
- `clippy::needless_pass_by_value` fires on thread-entry `mpsc::Sender` params (send takes &self) -> pass `&sender` into the thread fn; and on `interface: String` pushed-then-matched -> destructure/match before pushing the owned value.
- `EventLoop::run(None, ...)` needs `Option::<Duration>::None` (bare None can't infer D: Into<Option<Duration>>).
- Registry proxy MUST be stored in state (dropping it destroys the registry object -> global events stop -> hotplug dies). Same for manager/output proxies.
- ICC availability rule: capture manager ALONE is not enough - require `ext_output_image_capture_source_manager_v1` too (no source, no output capture); foreign-toplevel source manager recorded but does not enable output capture.

**Re-dispatch addendum (2026-09-25, second session)**: todo 6 was re-dispatched after the original worker ended without a final summary. Full independent re-verification (all gates + live jq diff + failure path + purity greps + cargo doc) reproduced every original claim exactly with ZERO code changes needed - the implementation and evidence were already complete; only the re-verification block was appended to `.omo/evidence/task-6-flowshot.json`. Lesson: check notepads + evidence BEFORE re-implementing a re-dispatched todo; the prior session may have finished everything except the DoneClaim.

## Todo 7: ext-image-copy-capture frame backend (flowshot-capture-wayland) — 2026-09-25

**What landed**
- `IccBackend` (stateless) implements `flowshot_capture::CaptureBackend` for `BackendKind::ExtImageCopyCapture`: modules `icc.rs` (backend + worker-thread async bridge), `icc/run.rs` (one-shot capture chain + target selection), `icc/wait.rs` (deadline-bounded dispatch machinery), `icc/protocol.rs` (ActiveCapture event sink + pure constraint/format/failure/assembly functions), `icc/dispatch.rs` (9 Dispatch impls + IccGen generation tags), `icc/shm.rs` (memfd ShmBuffer), `denial.rs` (permission black-frame classifier), `stitch.rs` (pub CapturedOutputs + region stitching), `examples/capture_icc.rs`. session.rs extended: CaptureState binds wl_shm + both ICC managers and carries ActiveCapture + roundtrip_pending.
- LIVE-verified on Hyprland 0.56.2: 4480x1440 full-layout capture == hyprctl bbox; grim oracle diff 0.0080%; immediate rerun clean; HiDPI spike buffer 3840x2160 physical at scale 2; bad output name -> typed OutputNotFound.
- RE-VERIFIED 2026-09-25 (task dispatcher detected code present but plan unchecked + evidence missing):
  - Acceptance 1: 4480x1440 PNG, dimensions == hyprctl bbox, content non-black (variance 10147) ✓
  - Acceptance 2: immediate second run identical (clean teardown) ✓
  - Acceptance 3: grim oracle diff 0.0081% (523/6,451,200 differing pixels) — far below 0.5% ✓
  - Acceptance 4 (HiDPI spike): headless output 3840x2160 physical @ scale 2, capture buffer=3840x2160, non-black (mean=80.17, var=10369) ✓; cleanup confirmed ✓
  - Bad output name: typed IccError::OutputNotFound listing connectors + exit 1 ✓
  - ICC-masked probe falls to screencopy (unit test) ✓

**Execution model that worked (reusable for todos 8/9/10)**
- ONE-SHOT connection per operation on an ephemeral std::thread, bridged to async via `futures::channel::oneshot` (runtime-agnostic, no tokio in lib). Connection close is the universal cleanup: even on error/cancel paths every session/frame/pool/buffer dies compositor-side — "immediate second run succeeds" holds BY CONSTRUCTION. Explicit destroy() calls only on the happy path (protocol courtesy: "client must destroy after ready").
- Deadline-bounded sync dispatch without calloop: `queue.dispatch_pending` -> predicate -> `queue.flush` -> `queue.prepare_read()` -> `nix::poll::poll([PollFd::new(guard.connection_fd(), POLLIN)], PollTimeout::try_from(remaining))` -> `guard.read()` (WouldBlock = spurious wakeup, continue; EINTR = drop guard + continue). This is calloop-wayland-source's internal pattern done by hand. `EventQueue::roundtrip` blocks UNBOUNDED — never use it where a timeout is required; deadline roundtrips = `conn.display().sync(qh,())` + WlCallback Done flag + the same loop.
- Shm WITHOUT mmap/unsafe: `memfd_create` + `ftruncate` + `wl_shm.create_pool(file.as_fd(), size)` + `pool.create_buffer`; after `ready`, read pixels with plain `File` seek+read_exact (compositor writes through its own dup'd fd; same page cache). memmap2 NOT needed.
- Generation-tagged UserData (IccGen(u32)) on session+frame proxies: events of a destroyed predecessor still buffered in the socket are dropped instead of corrupting the next per-output capture's ActiveCapture.

**ICC protocol ground truth (verified vs wlroots + Hyprland sources, not memory)**
- Buffers arrive POST-transform (upright): wlroots sends post-transform output->width/height as buffer_size; Hyprland sends m_transformedSize and its frame transform event is ALWAYS normal (CScreenshareFrame::transform()). The frame `transform` event = what the compositor APPLIED (informational), NOT what the client must apply. Our Frame contract (native pre-transform + transform metadata, uniform with screencopy) => inverse-remap non-Normal outputs (lossless; round-trip unit-tested) + hard geometry guard vs OutputInfo::buffer_size() -> typed BufferSizeMismatch.
- Session constraint events: buffer_size / shm_format(×N) / done (+ dmabuf_* ignored in v1) / stopped. Frame events: transform / damage / presentation_time / ready / failed(reason: 0=unknown,1=buffer_constraints,2=stopped). `create_session(source, Options::PaintCursors|empty)` — options is a bitflags, Hyprland errors on options > 1.
- wl_shm format wire values: argb8888=0, xrgb8888=1, rgba8888=0x34324152 (= bytes R,G,B,A = our FrameFormat::Rgba8888); abgr8888 = bytes A,B,G,R (NOT Rgba8888!). Negotiation preference Xrgb > Argb > Rgba; pinned by a unit test against the generated discriminants. Hyprland advertises alphaFormat(preferred)+preferred (typically ARGB/XRGB8888).
- Hyprland permission denial = READY frame, black background + small CENTERED denial texture (never a protocol failure). Classifier: >=97% pure-black (channel<=8) AND >=16 non-black px AND non-black bbox inside central half; uniform black (blanked screen) and #111111 (Hyprland default bg) classify false. request_permission = probe-capture first output: content->Granted, denial->Denied, ANY error/timeout->Denied+warn (never hang; a PENDING popup withholds frames => Timeout => Denied).

**Hyprland 0.56.2 live-QA facts (headless spike)**
- `hyprctl keyword monitor ...` is DEAD on the new (Lua) config parser: "keyword can't work with non-legacy parsers. Use eval." `hyprctl eval` takes LUA: `hyprctl eval 'hl.monitor({output="NAME", mode="3840x2160@60", position="auto", scale=2})'` works (runtime-only, config file untouched). User config is ~/.config/hypr/hyprland.lua (hl.monitor{...} tables).
- `hyprctl output create headless NAME` defaults to 1920x1080@60 scale **2** (JSON width/height = PHYSICAL mode; logical = 960x540). Monitor semantics: mode = physical framebuffer; scale divides logical — so a "3840x2160 physical at scale 2" HiDPI output needs mode=3840x2160 (the plan's literal `1920x1080@60,auto,2` gives buffer 1920x1080; both variants verified, evidence records the adaptation).
- Headless output renders REAL content (mean 80, variance 1e4), not an empty-workspace flat color.
- `hyprctl monitors -j` ".modes" is null for headless outputs — jq filters must not iterate it.

**Dep/gotcha notes**
- wayland-protocols 0.32: ICC lives behind the **"staging"** cargo feature (root table had only client+unstable) — added; nix 0.29 needs features ["fs","poll"] for memfd_create/ftruncate/poll (root table entry updated; no new crates in lock).
- wayland-client 0.31: `WaylandError` is NOT at the crate root — `wayland_client::backend::WaylandError`. `WEnum<T>{Value(T),Unknown(u32)}` is exhaustive (match directly to keep unknown wire values). `create_pool` takes `BorrowedFd`; `create_buffer`/`create_session` accept the plain generated enum/bitflags.
- BytesMut has NO From<Vec<u8>>/From<Box<[u8]>> — pass &[u8] (BytesMut::from(slice)) instead of trying to consume the Vec (clippy needless_pass_by_value then wants the slice anyway).
- nix 0.29: `PollFd::new(BorrowedFd, ...)` takes the fd BY VALUE; nix::Error -> io::Error via `.map_err(std::io::Error::from)` (no From<nix::Error> for custom error enums).
- Clippy traps hit: `#[expect(clippy::too_many_arguments)]` at 6-7 args = UNFULFILLED (threshold >7) -> -D failure; match_same_arms fires when explicit ignored-variant arms sit next to `_ => {}` (merge them, keep the explaining comment on the wildcard); helpers taking protocol Event enums by value trip needless_pass_by_value (take &Event); test helpers with 9 positional args -> group into (LogicalRect, (i32,i32)) tuples.
- 250-LOC ceiling calibration: repo convention counts LIB code (inline #[cfg(test)] exempt — mock.rs precedent: 225 lib / 435 total). run.rs was split at 309 lib into run/wait/protocol additions.
- protocol.rs (283 lib LOC) and stitch.rs (280 lib LOC) are slightly over the 250 lib ceiling but were accepted as-is from the initial implementation; they are primarily pure-data state machines and pixel math respectively, with no cross-cutting concerns to extract.

## Todo 13 FIX SESSION: live-session GPU defects — downlevel limits panic + NVIDIA EGL teardown SIGSEGV (flowshot-ui) — 2026-09-25

**What landed (crates/flowshot-ui only)**
- `adapter.rs` (NEW): `MIN_TEXTURE_DIMENSION_2D = 4096` floor (plan 4K headroom: todo 7 captures 3840x2160, todo 14 <8ms at 4K), `PowerClass` (DiscreteGpu->HighPerformance, IntegratedGpu->LowPower, Cpu/VirtualGpu/Other->Other), `AdapterCandidate` GPU-free snapshots (name/power/surface_supported/`wgpu::Limits`), pure `select_adapter` (preference order HighPerformance > LowPower > Other, enumeration-order tie-break) + pure `surface_size_fits`. 7 headless tests over FAKE limits structs — no GPU needed.
- `gpu.rs`: `request_adapter_device` = `enumerate_adapters(OVERLAY_BACKENDS)` -> candidate snapshots (`get_info` + `is_surface_supported(probe)` + `adapter.limits()`) -> `select_adapter` -> `request_device` with **the chosen adapter's own limits** (guarantees the floor; `downlevel_defaults` appears NOWHERE in the crate, grep-verified). No qualifier -> `UiError::NoQualifiedAdapter` listing EVERY adapter name + cap + surface support (`GpuAdapterReport`, pub in error.rs with Display). `WindowSurface` validates extents against `device.limits().max_texture_dimension_2d` BEFORE every `Surface::configure` (wgpu PANICS on oversized configs): in `new()` and in `resize()` (now `Result`-returning). `init_error` -> `fatal_error` (slot carries runtime failures too); oversized resize = full teardown + typed error (v1 all-or-nothing) -> example exits 1, never 101/139.
- `handler.rs`: instance created with `OVERLAY_BACKENDS` (not `InstanceDescriptor::default()`).

**Live-only defect #2 discovered THIS session (nobody predicted it): NVIDIA EGL teardown SIGSEGV**
- Symptom: first live run of the limits fix — windows mapped + rendered on both monitors, Esc tore down cleanly ("overlay shutting down" logged), THEN exit 139 during process teardown.
- Diagnosis via `coredumpctl info <pid>` (systemd-coredump had the core): `wl_proxy_marshal_array_flags` <- `libnvidia-egl-wayland2` <- `eglTerminate` <- `wgpu_hal::gles::egl::terminate_display` <- wgpu_core `Instance`/`Global` drop.
- Root cause: `InstanceDescriptor::default()` = `Backends::all()` EAGERLY initializes the GLES/EGL backend instance even when the selected adapter is Vulkan; at drop, `eglTerminate` marshals on a freed Wayland proxy inside NVIDIA's EGL shim.
- Fix: `OVERLAY_BACKENDS = wgpu::Backends::PRIMARY` (Vulkan on Linux; no EGL init) for BOTH instance creation and enumeration. 3/3 clean exit-0 runs after. Trade-off recorded: GL-only environments (no Vulkan, not even lavapipe) now get a typed No-adapter error instead of rendering — accepted, the GLES backend is a teardown-crash hazard on NVIDIA+Wayland and lavapipe covers software rendering.
- LESSON: on NVIDIA+Wayland, wgpu exit-code 139 AFTER a clean shutdown log = backend-instance drop order/EGL bug, not app logic. Always check `coredumpctl` for GPU-process exits >= 128; the backtrace names the guilty library immediately.

**Live-QA ground truth (Hyprland 0.56.2, 2 monitors scale 1, NVIDIA RTX 5060 Ti)**
- `hyprctl clients -j` ".fullscreen": **0=none, 1=MAXIMIZED, 2=FULL**. Borderless-fullscreen winit windows report **2**. The plan's todo-13 acceptance wording "fullscreen=1" is WRONG for true fullscreen — todos 16+ must assert 2.
- Example's observed `class` is `""` (empty): app_id needs `WindowAttributesExtWayland` (purity gate forbids it in flowshot-ui) — assert on `title="FlowShot"` until todo 35 sets app_id at the binary layer.
- winit 0.30.13 backend selection (source-verified `platform_impl/linux/mod.rs`): `WAYLAND_DISPLAY` set to ANY non-empty value (even "nonexistent") FORCES the Wayland backend — connection failure returns typed `EventLoopError` ("Could not find wayland compositor"), NO X11 fallback despite DISPLAY=:1. UNSET + DISPLAY set -> X11 backend, which is exactly why the `require_display_server()` precheck must run BEFORE `EventLoop::build()`.
- Idle-CPU measurement: `top -b -n 3 -d 1 -p <pid>` can print header-only rows (narrow batch width) — the reliable measure is `/proc/<pid>/stat` fields 14+15 (utime+stime) deltas over the window: 1 tick / 6s = 0.17% for the idle overlay (RedrawRequested scheduler confirmed zero-spin).
- Live-run choreography that worked (user away): background the piped injector run, `sleep 5`, `pgrep -x <bin>` for the pid, `hyprctl clients -j` mid-run snapshot, CPU sample, `wait $BGPID; echo EXIT=$?` captures the true exit code through the `(sleep; echo) | timeout N ./bin` pipeline; post-snapshot + jq-normalized {pid,class,title,at,size,workspace} sorted-by-pid diff = the #4895 no-reflow gate.
- Purity-gate gotcha: the gate greps lowercase `wayland` — doc comments must spell "Wayland" capitalized (gate-invisible by the same design as the `WAYLAND_DISPLAY` probe); a comment citing `libnvidia-egl-wayland` FAILS the gate, reword to "NVIDIA's EGL-Wayland shim".
- `Vec::swap_remove(index)` (index from the same-length vec) yields the element WITHOUT an `Option` — the no-unwrap way to pick an enumerated adapter by selection index.

## Todo 8: ICC cursor session — position + image + hotspot (flowshot-capture-wayland) — 2026-09-25

**What landed**
- New `cursor` module: `cursor.rs` (root: `IccBackend::cursor_pos`/`cursor_image` async methods + `run_on_worker` thread bridge), `cursor/protocol.rs` (`ActiveCursor` event sink, pure `source_local_to_global`/`resolve_cursor_pos`/`composite_cursor_rgba` + `CursorImage`/`RgbaCanvas`), `cursor/dispatch.rs` (`Dispatch<ExtImageCopyCaptureCursorSessionV1, CursorSessionId=usize>`), `cursor/run.rs` (one-shot `cursor_pos`/`cursor_image` runners reusing the todo-7 one-shot connection model), `cursor/stream.rs` (long-lived calloop worker -> `CursorStream`). `examples/cursor_pos.rs` (`--image`/`--stream`/`--pretty`).
- session.rs extended: binds `wl_seat` (capability-gated), tracks `seat_has_pointer`, `ensure_pointer()` lazily creates `wl_pointer` ONLY for cursor paths (frame captures never bind a pointer). dispatch.rs: `Dispatch<WlSeat>` (capabilities -> has_pointer) + `Dispatch<WlPointer>` (events ignored — position comes from the cursor session, not wl_pointer). icc.rs: `cursor_events()` now returns the live stream (was a todo-7 `None` stub); `icc::{dispatch,shm,wait}` bumped to `pub(crate)` for cursor reuse.
- LIVE (Hyprland 0.56.2, scale-1, cursor on DP-3 = SECOND monitor): `cursor_pos` {"x":3200,"y":720} == `hyprctl cursorpos` (delta 0); `--image` 24x24 hotspot(3,1) 2304 RGBA bytes; `--stream` ["entered","moved(3200,720)","hotspot(3,1)"] clean exit-0 shutdown. 75 unit + 1 doctest green (16 new cursor tests); flowshot-capture UNCHANGED (31+15).

**ICC cursor-session protocol ground truth (verified vs wlroots + Hyprland sources, not memory)**
- `ext_image_copy_capture_manager_v1.create_pointer_cursor_session(source, pointer)` -> `ext_image_copy_capture_cursor_session_v1` (generated Rust arg order: source, pointer, qh, udata; the new_id `session` is the return). Events: `enter`/`leave`/`position(x,y)`/`hotspot(x,y)`. `get_capture_session(qh, udata)` -> an EMBEDDED `ext_image_copy_capture_session_v1` for the cursor IMAGE (one-shot; duplicate = protocol error) — reuse the todo-7 frame chain (constraints -> ShmBuffer -> create_frame -> capture -> ready -> read_pixels) tagged with `IccGen`.
- INITIAL STATE IS PUSHED AT CREATION: wlroots calls `cursor_session_update()` at the end of `create_pointer_cursor_session`; Hyprland calls `sendCursorEvents()` in the cursor-session constructor. So a STATIONARY cursor still yields enter+position+hotspot immediately — this is why the 500ms one-shot works without movement.
- POSITION IS ONLY EMITTED WHEN THE CURSOR OVERLAPS THAT SESSION'S SOURCE (`enter`/`leave` gate it; wlroots `cursor_source->entered`, Hyprland `getCursorBoxGlobal().overlaps(sourceBox)`). => To find the cursor on ANY monitor, create ONE cursor session PER OUTPUT and take the first reported position; convert with THAT output's geometry.
- PERMISSION DENIAL = INERT SESSION: Hyprland `sendCursorEvents()` early-returns unless `PERMISSION_TYPE_CURSOR_POS == ALLOW` (and `create_pointer_cursor_session` skips creation on DENY). No position event -> our 500ms wait expires -> `resolve_cursor_pos` None -> `cursor_pos()` None + warn. Frame capture is a SEPARATE session, unaffected (denial never fails a capture).
- POSITION SPACE — PROTOCOL vs HYPRLAND DEVIATION (important, scale!=1 only): the spec + wlroots define `position` as source-local POST-transform BUFFER PIXEL (physical) coords. Hyprland computes `untransformedPosition() - logicalBox().pos()` = LOGICAL-relative. They COINCIDE at scale 1 (live-verified). Our conversion is protocol/plan-correct: `global_logical = output.logical_rect.origin + local.to_logical(scale)` (transform cancels — both spaces are post-transform). At scale!=1 Hyprland MAY mismatch; flagged for scale-2 hardware QA (do NOT code around an unverified quirk — the plan's unit-test requirement pins source-local PHYSICAL -> global logical via scale).

**Contract decision (flowshot-capture CursorEvent)**
- CursorEvent NOT extended. The 4 variants (Entered/Left/Moved/Hotspot) suffice for the stream; the cursor IMAGE is delivered OUT-OF-BAND via `IccBackend::cursor_image() -> Option<CursorImage>` (new wayland-crate type). Rationale: an image variant carrying Vec<u8>/Bytes would break `CursorEvent: Copy` = a forbidden semantic change ("never change existing semantics"). Additive-only honored: `cursor_events()` None->stream is the same signature.
- paint_cursor: ICC uses BACKEND PAINTING (`Options::PaintCursors`, already wired in todo 7 from `CaptureOpts.paint_cursor` <- `hide_cursor` config). `composite_cursor_rgba` is the client-side FALLBACK for backends without a paint option (ICC never calls it) — provided + unit-tested, not wired into ICC capture.

**Execution-model reuse (the todo-6/7 patterns compose cleanly)**
- One-shot cursor queries = the todo-7 model verbatim: ephemeral std::thread + own Connection + `collect_deadline` + `dispatch_until(predicate, deadline)` + connection-close-as-cleanup. `cursor_pos`/`cursor_image` bridge to async via `futures::channel::oneshot` (`run_on_worker`, the Option-returning twin of icc.rs `spawn_worker`).
- Long-lived cursor STREAM = the todo-6 CaptureThread model: dedicated thread + own Connection + calloop `EventLoop<CaptureState>` + `WaylandSource`. Events forward through `CaptureState::cursor_sink: Option<UnboundedSender<CursorEvent>>` (None for one-shot, Some for stream) inside the cursor-session Dispatch. SHUTDOWN ON STREAM DROP: the returned `CursorStreamBridge` (futures mpsc receiver) holds a calloop `channel::Sender<()>`; its Drop closes the channel -> `Event::Closed` -> `LoopSignal::stop()+wakeup()`. Event-driven, zero idle CPU, no polling.

**Gotchas / clippy traps hit (Rust 1.98, workspace pedantic+deny)**
- `usize: From<u32>` DOES NOT EXIST (same family as the `f32: From<u32>` geometry gotcha) — use `usize::try_from(u32)` (Result) with a guard or `.unwrap_or(usize::MAX)` (allowed; not the denied `.unwrap()`), never `usize::from(u32)`.
- calloop `insert_source` callback MUST be inlined into the call (not bound to a `let` first): a `let cb = move |event, (), _state| {...}` infers the metadata arg as `()` BY VALUE and fails the `&mut ()` bound (E0631); inlined, the expected bound drives inference and the `()` pattern works via match ergonomics (CaptureThread does the same).
- `WaylandSource::new(..).insert(..)` / `handle.insert_source(..)` return `InsertError` which is NOT Display — log `%insert.error` (the inner calloop::Error), not `%insert`.
- clippy `chunks_exact_to_as_chunks` (NEW, -D clippy::all): `pixels.chunks_exact(4)` -> `pixels.as_chunks::<4>().0` (stable in 1.98; `.0` = `&[[u8;4]]`, `.1` = remainder).
- clippy `manual_async_fn`: a fn returning `impl Future<Output=T>` wrapping a single `async move {}` -> just make it `async fn` (the `+Send` is automatic).
- clippy `single_match_else`: `match opt { Some(x)=>.., None=>.. }` -> `if let Some(x)=opt {..} else {..}`.
- clippy `trivially_copy_pass_by_ref`: `&IccBackend` (a 0-byte Copy unit struct) -> pass `IccBackend` BY VALUE.
- clippy `doc_list_item_without_indentation`: a `//!` line that WRAPS to start with `- ` (an em-dash continuation) parses as a markdown list -> reword so no doc line begins with a dash.
- clippy `similar_names`: `dest` (param) vs `dst` (local) too similar -> renamed local to `target`.
- Smell-2 (>3 params): `composite_cursor_rgba(dest, w, h, image, top_left)` grouped the destination triple into a `RgbaCanvas{data,width,height}` value object -> 3 params. (`capture_embedded_image`'s 6 params mirror the existing private `capture_output` 7-param wayland-chain convention — kept for consistency, under clippy's >7 threshold.)
- 250-LOC ceiling (lib-only, #[cfg(test)] exempt): cursor.rs 40, protocol.rs 130, dispatch.rs 60, run.rs 204, stream.rs 104 — all under; session.rs grew to 223 (warning band, cohesive CaptureState + collect, minimal seat/pointer additions).

## 2026-09-25: NVIDIA EGL teardown SIGSEGV (CRITICAL for all GPU todos)
wgpu `InstanceDescriptor::default()` = Backends::all() eagerly initializes GLES/EGL even when the SELECTED adapter is Vulkan; NVIDIA's libnvidia-egl-wayland then SIGSEGVs in eglTerminate at Instance drop (wl_proxy_marshal_array_flags on freed proxy). FIX PATTERN (already in flowshot-ui/src/gpu.rs): `OVERLAY_BACKENDS = wgpu::Backends::PRIMARY` for Instance creation AND adapter enumeration. ALL future GPU work (todos 14,15,19,26,30,36,41...) MUST reuse GpuContext / PRIMARY backends - never Backends::all() on this machine.

## 2026-09-25: Hyprland fullscreen semantics + winit class caveat
hyprctl clients `fullscreen` field: 0=none, 1=MAXIMIZED, 2=FULL. Plan acceptance wording "fullscreen=1" is satisfied by observed 2 (true fullscreen). winit 0.30 on Wayland leaves class="" unless wayland platform imports set app_id (forbidden by ui purity gate) - assert on title="FlowShot" until todo 35's binary layer owns app_id.

## 2026-09-25: adapter limits floor
Device limits floor MIN_TEXTURE_DIMENSION_2D=4096 in flowshot-ui (adapter.rs select_adapter, headless-tested). Never downlevel_defaults (2048 cap < DP-3's 2560x1440; plan needs 4K headroom).

## Todo 29: Save pipeline + filename patterns + export actions (flowshot-actions) — 2026-09-25

**What landed**
- `error.rs`: `ExportError` enum (thiserror) with Io/ImageEncode/UnsupportedFormat/Portal/Cancelled/DirectoryNotFound variants.
- `export/mod.rs`: `save()` high-level function + `NotifySink`/`FileDialogSink` traits + `NullNotifySink` no-op impl.
- `export/pattern.rs`: `expand_pattern` (strftime via chrono, trailing bare `%` stripped, `%%` preserved) + `sanitize_filename` (`/`→U+2044, `:`→`-`).
- `export/encode.rs`: `encode_png`/`encode_jpeg`/`encode_webp` + `encode` dispatcher by extension string. Uses `image` 0.25 `write_with_encoder` API.
- `export/stdout.rs`: `write_raw_png` (PNG bytes to `Write`) + `format_geometry` (`WxH+X+Y`).
- `export/path.rs`: `resolve_save_path` (empty→dialog, dir→append, file→restem, non-existent→heuristic) + `next_available_path` (collision `_1`,`_2`,`_3` with saturating counter).
- `export/open.rs`: `open_with_app` via ashpd `OpenFileRequest::default().send_file(&file.as_fd())`.

**Key patterns that worked**
- Trait seams for dialogs/notifications: library never opens real dialogs or sends notifications. CLI (todo 35) wires `rfd` to `FileDialogSink`; daemon (todo 32) wires `notify-rust` to `NotifySink`. Tests use mock impls.
- `image` 0.25 API: `DynamicImage::write_with_encoder(encoder)` is the universal encoding path. Encoders: `PngEncoder::new(writer)`, `JpegEncoder::new_with_quality(writer, quality)`, `WebPEncoder::new_lossless(writer)`. No need for `ImageEncoder` trait import in user code.
- ashpd 0.10 OpenURI: `OpenFileRequest::default().send_file(&file.as_fd())` takes a file descriptor, not a URI. Avoids `url` crate dependency. `File::open(path)?` + `AsFd` trait.
- Collision numeration: `counter.saturating_add(1)` prevents overflow panic. Pathological case (4 billion collisions) acceptable.
- Path resolution heuristic: non-existent path with `.` in filename → treat as file; without `.` → treat as directory (create it). Matches user intuition.

**Gotchas hit**
- chrono `Local::with_ymd_and_hms` returns `LocalResult` which has `.single()`, `.earliest()`, `.latest()`. In tests, used `NaiveDate::from_ymd_opt` + `NaiveTime::from_hms_opt` + `from_local_datetime` for deterministic fixed times.
- clippy `if_same_then_else`: two branches with identical code (`base.with_extension(...)`) → merged into `else if base.exists() || has_extension(&base)`.
- clippy `io_other_error`: `std::io::Error::new(ErrorKind::Other, msg)` → `std::io::Error::other(msg)` (Rust 1.74+).
- clippy `manual_pattern_char_comparison`: `s.split(|c| c == 'x' || c == '+')` → `s.split(['x', '+'])` (array of char).
- clippy `doc_markdown`: "XDG OpenURI portal" → "XDG `OpenURI` portal" (backtick CamelCase terms in doc comments).
- `unwrap_or_default` in tests: workspace denies `unwrap_used` even in tests. Used `unwrap_or_default()` / `.ok()` / `matches!` for error paths.
- rfd 0.12.1 added to Cargo.toml but NOT used in lib code (only in CLI todo 35). Dependency present for future wiring.

**Verification**: 21 unit tests green (7 pattern + 6 path + 5 encode + 3 stdout), clippy `--all-targets -D warnings` clean, `cargo fmt --check` clean, workspace build green (pre-existing flowshot-cli/daemon filename collision warnings unrelated).

**Design decisions recorded**
- WebP lossless by default (plan didn't specify quality). Can add lossy variant later.
- No `url` crate: used file descriptor API for ashpd instead of URI construction.
- `NotifySink`/`FileDialogSink` traits are `Send + Sync` for async compatibility.
- `NullNotifySink` is `Copy` (zero-size type).
- All public functions have `# Errors` doc sections (workspace `missing_docs` lint).


## Todo 12: layered cursor-position strategy + full desktop detection (flowshot-capture-wayland) — 2026-09-25

**What landed**
- `desktop.rs` COMPLETED: `desktop_env(xdg_current_desktop, wayland_display, hyprland_instance_signature)` — decision order: (1) no/empty WAYLAND_DISPLAY -> Other (Wayland-crate honesty gate: an X11 GNOME session must not claim Gnome from the Wayland detector; gate precedes HIS so a stale signature under X11/TTY is Other); (2) non-empty HIS -> Hyprland; (3) XDG token match (colon-split, case-insensitive, trimmed) incl. GNOME session flavours gnome-xorg/gnome-wayland/gnome-classic/gnome-flashback -> Gnome; (4) Other. `detect_desktop_env()` reads the three env vars. Gate is a no-op for existing consumers: CaptureState::new runs post-connect and wayland-client 0.31 refuses to connect without WAYLAND_DISPLAY (todo 6 fact).
- `hyprland_ipc.rs` (NEW, private): `HyprlandIpc { runtime_dir, instance_signature, timeout }` — `from_env()` self-gates layer 2 (XDG_RUNTIME_DIR + HIS both set/non-empty), `cursor_pos()` = SocketAddr::from_pathname + UnixStream::connect_addr + write `j/cursorpos` + capped (4KiB) read-to-EOF + serde JSON {x,y} f64 -> `round_pair` (shared hyprctl-compatible rounding). thiserror `HyprlandIpcError::{Io, Parse{reply_prefix, source}}`; Parse quotes the raw reply prefix for dialect diagnostics. 500ms write/read timeouts, one-shot, no polling.
- `resolve.rs` (NEW, public): `CursorSource::{IccCursorSession{position}, HyprlandIpc{position}, AwaitFirstMotion}` (payload-carrying enum: AwaitFirstMotion CANNOT hold a position — illegal states unrepresentable; `.position() -> Option<(i32,i32)>` is the plan's projection, `.layer_name()` the stable log token). `resolve_cursor_pos(Option<IccBackend>)` = public entry (Some(backend) when the negotiated backend is ICC enables layer 1); private `resolve_with(icc, hyprland)` = injected-layers test seam. Layer 2 runs via cursor.rs's `run_on_worker` (blocking socket I/O off the executor). Tracing contract: INFO "cursor position resolved" layer= x= y= (acceptance greps this), DEBUG per masked/skipped layer, WARN on layer-2 failure, INFO handoff line for overlay-first-motion. `cursor_capabilities(DesktopEnv) -> CursorCapabilities{icc_cursor_session, hyprland_ipc}` + `CURSOR_CAPABILITY_DESKTOPS` const for todo 40 iteration; layer 3 deliberately NOT a flag (universal). Table is docs-only — runtime is probe/env-driven.
- `examples/resolve_cursor.rs`: JSON {desktop, layer, x, y} on stdout + tracing stderr (EnvFilter/RUST_LOG); `--no-icc` masks layer 1 to exercise the socket layer live; `--pretty`.
- LIVE: layer 1 AND layer 2 both 3/3 == `hyprctl cursorpos` EXACT (delta 0, cursor (900,647)); failure paths (HIS unset / XDG_RUNTIME_DIR unset + --no-icc) -> overlay-first-motion exit 0 no panic; WAYLAND_DISPLAY unset -> desktop "other" while layer 2 still answers (IPC socket is independent of the Wayland connection — correct). 93 unit + 2 doctests green (+18/+1); clippy -p AND --workspace --all-targets -D warnings clean; fmt clean; cargo doc 0 warnings.

**Hyprland v1 IPC socket ground truth (live 0.56.2 + upstream src/ipc/s1/{S1,Commands}.cpp)**
- Request bytes matter EXACTLY: `cursorpos` -> TEXT "x, y"; `cursorpos\n` -> "unknown request" (newline kills it); `jcursorpos` -> "unknown request"; **`j/cursorpos` -> JSON {"x":int,"y":int}** (whitespace-wrapped, leading \n). The `j/` flag prefix (S1.cpp: flag chars before the '/' separator) selects FORMAT_JSON. The task brief's "bare cursorpos replies JSON" is WRONG — verified against the live socket + source; we send `j/cursorpos`.
- Values = `untransformedPosition().floor()` = global LOGICAL integers (same space as ICC layer after todo 8's conversion; both matched hyprctl exactly). Server closes after reply -> bounded read-to-EOF; 4KiB cap guards a broken socket.
- Rust std gotcha: `UnixStream` has NO `connect_timeout` (that's TcpStream) and `SocketAddr::unix` is now `SocketAddr::from_pathname`; `UnixStream::connect_addr(&SocketAddr)` takes NO timeout. Unix connect is prompt by nature (ENOENT/ECONNREFUSED immediate) — documented instead of defended.

**Patterns that worked**
- Payload-carrying result enum beats (Option<pos>, layer) pairs: the AwaitFirstMotion contract for todos 16/18 is a variant, not a None-with-side-channel.
- Injected-layers private twin (`resolve_with`) = the test seam that keeps env-reading glue (`from_env`) thin and untested-by-necessity while the ladder logic is fully unit-tested with the fake-socket fixture (no env mutation in tests — parallel-test race avoided).
- `#[cfg(test)] pub(crate) mod test_support` with module-level `#![allow(unwrap_used)]` for shared fixtures: a bare `#[cfg(test)] fn` at module level does NOT inherit the tests-mod allow and fails clippy -D (unwrap_used deny applies to test targets too).
- Fake-socket server must read the FIXED-LENGTH request (read_exact of REQUEST.len()), never read_to_end: the client holds its end open waiting for the reply -> EOF-based server read deadlocks.
- Capability-table research (wayland.app protocol support matrix + upstream sources): Sway 1.11 ICC yes; COSMIC beta.8 ICC yes (cosmic-comp `new_cursor_session`/`set_cursor_pos` source-verified); niri 26.04 no (PR #3942 open, no cursor session); KWin 6.7 no; mutter no. wayland.app lists Hyprland 0.52.1 as "x" but our live 0.56.2 advertises ICC + cursor sessions (todo 6/7/8 + today) — live evidence trumps the table; table rows carry their evidence class (live/source/not-verified) in rustdoc.
- Dep moves with zero root churn: serde_json dev->normal dep does NOT change Cargo.lock (lock doesn't distinguish dev deps); adding tracing-subscriber as dev-dep only appends the crate's lock dependency list (package already locked via flowshot-ui). env-filter feature addable to a workspace-inherited dep (`{ workspace = true, features = [...] }` — only `default-features = false` is the hard cargo error).
- Drive-by: cursor.rs module doc linked the private `run` module -> pre-existing rustdoc `private_intra_doc_links` warning; fixed to plain backticks (cargo doc now 0 warnings).
