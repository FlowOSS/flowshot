# Refactoring & deduplication opportunities

Research report, 2026-09-29. Analysis only — nothing was implemented. Scope:
all seven crates (`crates/*/`). Method: whole-tree pure-LOC measurement
(non-blank, non-comment lines; production = everything before the inline
`#[cfg(test)] mod` block), four parallel deep-dive audits (capture-wayland
backends, settings/window shells, errors/fixtures/constants,
pins/widgets/routing), and manual verification of every cited claim
(`diff`-checked the top candidates line by line).

## TL;DR

The codebase is **already well factored**. There is no monolith to dismantle
and no architecture-level duplication: the shared capture infrastructure
(`session.rs`, `worker.rs`, `stitch.rs`, the `_with::<E>` generics), the
`render/` display-list layer, the `widgets/` kit, the single input funnel, and
the settings `form.rs`/`fields.rs` row builders all already exist and are used
correctly. The speculative-abstraction audit found **zero traits to delete** —
every one of the 18 traits has 2+ implementers or is a documented crate
boundary, test seam, or user-mandated plugin point.

What is winnable:

- **~700 production LOC** (~1.8% of the 38,254 production pure-LOC) across 10
  concrete merge items, ranked below. The two big ones: the near-identical
  settings/launcher egui window shells (~250 LOC; the 60-line `spawn`
  functions differ in **4 lines**) and the copy-shaped backend error enums
  (~200 LOC; the in-tree `portal_backend_error!` macro already proves the
  consolidation pattern).
- **~200 test LOC** (optional tier): a `flowshot-testsupport` dev-dep crate
  for the 27 hand-rolled `OutputInfo` builders and 4 `OutputLayout` builders.
- **11 files over the 250-LOC production ceiling**, with split suggestions.
  Splits are LOC-neutral; the ceiling is an actively enforced project
  convention (see `daemon/src/execute/session/exit.rs:1-3`: "Extracted from
  [`super`] at the 250-LOC ceiling").

Honest floor: tests + QA harnesses measure 31,470 pure LOC (12,719 inline +
18,751 dedicated test/example files) and mostly **should stay** — they are the
project's QA story (draft D4: "test the core, QA the edges"; 1,218 green
tests). The winnable delta lives in the 38,254 production lines, and it is
~700 LOC, not thousands.

## Measurements

Pure LOC (non-blank, non-comment), measured per file with the inline-test
split at the first `#[cfg(test)] mod X {` body:

| Region | Pure LOC |
|---|---|
| Production code (`crates/*/src`, excl. inline tests) | 38,254 |
| Inline `#[cfg(test)]` modules | 12,719 |
| Dedicated test files + examples (`tests/`, `examples/`, `*tests.rs`, stubs, mocks) | 18,751 |

(Consistent with `docs/codebase-comparison.md`'s ~37.3k production estimate;
the small delta is counting method — pure LOC here vs tokei "Code" there, and
examples are included in the test/harness bucket here.)

## Ranked opportunities

Ranked by LOC-saved-per-risk. "Prod" = production-code savings.

| # | Item | Where | LOC | Risk | Verdict |
|---|---|---|---|---|---|
| R1 | Backend error enums → one macro | capture-wayland `error.rs`, `kwin/error.rs` | ~200 prod | Low | REFACTOR |
| R2 | Merge settings + launcher egui window shells | ui `settings/window/*`, `launcher/*` | ~250 prod | Low-Med | REFACTOR |
| R3 | `to_native_orientation` ×3 → one generic fn | icc/kwin/portal | ~60 prod | Low | REFACTOR |
| R4 | kwin copies portal's D-Bus plumbing | `kwin/run.rs` ← `portal/run.rs` | ~55 prod | Low | REFACTOR |
| R5 | `local_*` coord helpers ×2 | ui `editor/paint.rs`, `selection/paint.rs` | ~50 prod | Low | REFACTOR |
| R6 | Exit-code mapping duplicated CLI↔daemon | `cli/exit.rs`, `daemon/…/session/exit.rs` | ~35 prod | Low | REFACTOR |
| R7 | `select_targets`/`named_target` ×2 | `icc/run.rs`, `screencopy/run.rs` | ~30 prod | Low | REFACTOR |
| R8 | `to_rgba` ×2 → `FrameFormat` method | `stitch.rs`, ui `backdrop/pixels.rs` | ~15 prod | Low | REFACTOR |
| R9 | `wl_shm::Format`→`FrameFormat` map ×2 | `icc/protocol.rs`, `screencopy/protocol.rs` | ~12 prod | Low | REFACTOR |
| R10 | `LINE_HEIGHT_RATIO` ×2 | `editor/paint.rs`, `chrome/hud.rs` | ~2 prod | Trivial | REFACTOR |
| R11 | Shared test-fixture crate (optional tier) | 27+ builders across 4 crates | ~200 test | Med | REFACTOR (optional) |
| — | icc↔screencopy `capture_run` skeleton | `icc/run.rs`, `screencopy/run.rs` | (~80) | Med | REJECT |
| — | `CaptureBackend` impl skeleton ×4 | icc/screencopy/kwin/portal | (~180) | Med | REJECT |
| — | Central timeout vocabulary module | capture-wayland, actions | (~30) | — | REJECT |
| — | Settings form-row "row builder" | ui `settings/tabs/*` | 0 | High | REJECT |
| — | Event-routing unification (overlay/pins) | ui | 0 | High | REJECT |

**Total realistic savings: ~709 production LOC + ~200 test LOC (optional).**

## Per-item evidence

### R1. Backend error enums → one macro (~200 LOC, low risk) — REFACTOR

`IccError` (`capture-wayland/src/error.rs:84-168`) and `ScreencopyError`
(`error.rs:201-279`) share **15 identically shaped variants**: `Connect`,
`Collection`, `MissingProtocol`, `NoOutputs`, `OutputNotFound{requested,
available}`, `NoSupportedFormat`, `IncompleteConstraints`,
`BufferSizeMismatch{reported, expected}`, `Timeout{timeout}`, `Io`,
`Transport`, `Protocol`, `Geometry`, `PermissionDenied`, `Internal`. Only
`FrameFailed{reason}` vs `FrameFailed` (screencopy's wire protocol carries no
failure reason) and ICC's `SessionStopped`/`UnknownFailureReason` differ.
`KwinError` (`kwin/error.rs:95-173`) repeats 11 of the same variants;
`PortalErrorKind` (`portal/error.rs:40-206`) repeats 9.

The `→ CaptureError` lift (`Timeout → CaptureError::Timeout`, everything else
`→ CaptureError::Backend`) is written out identically three times
(`error.rs:171-183`, `error.rs:282-294`, `kwin/error.rs:181-193`) and
macro-generated twice (`portal/error.rs:275-287`).

The fix already exists in-tree: `portal_backend_error!`
(`portal/error.rs:212`) generates the variant table + `BackendError` impl +
`From` conversions + `CaptureError` lift for the two portal newtypes.
Generalizing it (shared variants macro-parameterized by backend-specific
extras) removes ~200 LOC of hand-maintained parallel tables.

Risk: macro-generated variants are harder to grep and jump-to-definition;
per-backend `Display` strings must stay distinct (they name the protocol);
`thiserror` inside `macro_rules!` needs hygiene care. All of this is already
solved by the portal macro — this is extending a proven pattern, not
inventing one. The per-crate typed-error boundary itself stays (Amendment #4:
thiserror in libs, no stringly errors across crate boundaries).

### R2. Merge the settings + launcher egui window shells (~250 LOC, low-med risk) — REFACTOR

The two egui-hosted, single-window, opaque dialog shells are near-identical.
Verified by direct diff: `settings/window/app.rs:56-111` vs
`launcher/app.rs:65-120` (`spawn`) differ in **4 lines out of ~60** (window
size + resizable flag). Line-for-line identical or trivially parameterizable:

| Piece | Settings | Launcher | Duplicated |
|---|---|---|---|
| `spawn` (window+instance+surface+gpu+egui) | `settings/window/app.rs:56-111` | `launcher/app.rs:65-120` | ~50 |
| `resize` (reconfigure + ppp/screen size) | `app.rs:113-153` | `launcher/app.rs:122-162` | ~35 |
| `ApplicationHandler` impl | `app.rs:156-217` | `launcher/app.rs:165-232` | ~50 |
| frame render (acquire→egui frame→paint→present→cursor→repaint delay) | `settings/window/frame.rs:17-104` | `launcher/frame.rs:16-100` | ~80 |
| runtime (display-server check→event loop→run→fatal) | `settings/window/runtime.rs:76-132` | `launcher/window.rs:151-210` | ~80 |

Real differences, all parameterizable: window attributes + surface label;
action dispatch (`FrameAction::Apply` + persistence vs `LauncherAction::Capture`
+ invoke-and-exit); launcher's `#[cfg(feature = "test-drive")]` synthetic-input
branch and monitor probe; settings' `SetSystemTheme` event. Merge shape: a
generic `EguiWindow<M, A, E>` shell (model, action, per-window event enum)
next to the already-shared `gpu.rs` (`GpuContext`, `configure_opaque_surface`)
and `egui_host/` (`EguiSurface`, input bridge, present helpers).

This is **same-intent** duplication: both surfaces are "egui dialog window",
both track egui/winit version bumps and the `GpuContext` API together. Risk:
the generic event enum + cfg-gated test-drive branch add one level of
indirection; if the launcher later grows dialog-specific behavior (e.g.
per-monitor previews) the shell must not constrain it — keep the per-window
`update`/`action` closures fat, the lifecycle thin.

Explicitly **not** part of this merge: the overlay (`app.rs`, `runtime.rs`,
`surface.rs` — multi-monitor, custom-rendered, transparent, frozen-backdrop)
and pins (`pins/shell.rs`, `pins/spawn.rs`, `pins/runtime.rs` — multi-window,
custom-rendered, state-machine routing). Those are different-intent lifecycles
(see do-NOT-merge #8).

### R3. `to_native_orientation` ×3 → one generic fn (~60 LOC, low risk) — REFACTOR

Identical inverse-remap + dimension-swap logic in three backends, differing
only in the error type:

- `icc/protocol.rs:252-283`
- `kwin/meta.rs:235-266`
- `portal/composite.rs:251-284`

All three error types already implement the crate's `BackendError` trait
(`error.rs:304-343`), so one `fn to_native_orientation<E: BackendError>(…) ->
Result<FrameBuffer, E>` in a shared module (e.g. next to `stitch.rs`) replaces
all three. Pure function; per-backend tests stay where they are. Low risk:
the orientation math is exactly the kind of thing that must NOT drift between
backends (a per-backend copy is a #4871-class bug waiting to happen).

### R4. kwin copies portal's D-Bus plumbing (~55 LOC, low risk) — REFACTOR

- `collect_outputs`: `kwin/run.rs:281-292` vs `portal/run.rs:85-96` —
  identical except error type and timeout constant.
- `select_outputs`: `kwin/run.rs:301-325` vs `portal/run.rs:148-172` —
  identical except error type.
- `with_deadline`: `kwin/run.rs:330-344` vs `portal/run.rs:102-116` — kwin's
  copy is **already generic** over `E: BackendError`; the only difference is
  the tracing message string.

Precedent exists: kwin already imports `build_runtime` from `portal::run`
(`kwin/run.rs:138`). Cleanest form: move the four functions into a neutral
shared module (e.g. `dbus_common.rs` or promote `portal/run.rs` →
`runtime.rs`) so kwin doesn't textually depend on "portal", generic over
`E: BackendError` with the timeout passed in.

### R5. `local_*` coordinate helpers ×2 (~50 LOC, low risk) — REFACTOR

Global-logical → window-local-physical conversion helpers duplicated between:

- `editor/paint.rs:250-268` — `local_rect`, `local_x`, `local_y`, `local_len`
  (`pub(crate)`/`pub(super)`)
- `selection/paint.rs:126-150` — same four + `local_point` (private)

Same signatures (`output: &OutputInfo`, …), same math (subtract output origin,
multiply by that output's own scale — the single conversion boundary the
physical-first rule mandates). Move into the existing `render/geom.rs` (or a
small `coords.rs`). `backdrop/scene.rs:55`'s `local_physical_rect` is the
**clamped** variant (dim-cutout algebra) — different semantics, stays.

### R6. Exit-code mapping duplicated CLI↔daemon (~35 LOC, low risk) — REFACTOR

`daemon/src/execute/session/exit.rs` self-describes as "The shared exit-code
mapping" — yet the CLI keeps a private copy instead of calling it:

- `code_for` (`daemon/…/session/exit.rs:9-21`, raw literals `2/5/4/6/1`) vs
  `exec_error_code` (`cli/src/exit.rs:93-107`, named constants) — same match.
- `is_permission_denied` — **verbatim identical** in both
  (`daemon/…/session/exit.rs:29-41`, `cli/src/exit.rs:112-124`), including the
  `IccError::PermissionDenied` source-chain walk.

The CLI already depends on `flowshot-daemon`. Fix: CLI's `exec_exit_u8` keeps
the `Ok` arms and delegates `Err(error)` to the daemon's `code_for`; the named
constants move to (or are re-exported from) the daemon module and the raw
literals there are replaced by them. Exit codes are a release contract —
`cli/tests/exit_mapping.rs` and the daemon suite already pin the table, which
is what makes this low-risk.

### R7. `select_targets`/`named_target` ×2 (~30 LOC, low risk) — REFACTOR

`icc/run.rs:139-162`/`169-184` vs `screencopy/run.rs:132-155`/`162-177`:
identical Selection→TrackedOutput resolution, differing only in the error
type. Same `Selection::All`/`Named` semantics as R4's `select_outputs` — one
generic resolver over `E: BackendError` can serve all four backends (the
Wayland-protocol pair resolves against `TrackedOutput`, the D-Bus pair against
`OutputInfo`; either two small generics or one over `OutputInfo` after
snapshot).

### R8. `to_rgba` ×2 → `FrameFormat` method (~15 LOC, low risk) — REFACTOR

`capture-wayland/src/stitch.rs:106-113` and `ui/src/backdrop/pixels.rs:43-48`
are identical, down to the test (`to_rgba_converts_every_v1_format` at
`stitch.rs:356` and `pixels.rs:240`). The mirror comment
(`backdrop/pixels.rs:38-39`) says the purity gate forbids importing "the
platform capture crate" — true for `flowshot-capture-wayland`, but the
function only needs `FrameFormat`, which lives in **`flowshot-capture`**
(`frame.rs:33-40`), an allowed `flowshot-ui` dependency (`ui/Cargo.toml:22`).
Move it to `FrameFormat::to_rgba(self, pixel: [u8; 4]) -> [u8; 4]`; both call
sites and one of the two identical tests disappear. The mirror comment's
rationale does not apply to the capture crate — this is a purity-gate
misreading frozen into a duplicate, not a deliberate seam.

### R9. `wl_shm::Format` → `FrameFormat` mapping ×2 (~12 LOC, low risk) — REFACTOR

`icc/protocol.rs:29-40` and `screencopy/protocol.rs:53-55` carry the same
Xrgb8888/Argb8888/Rgba8888 mapping. One small shared helper inside
capture-wayland (e.g. `formats.rs`). Trivial; do it alongside R3/R4.

### R10. `LINE_HEIGHT_RATIO` ×2 (~2 LOC, trivial) — REFACTOR

Canonical: `editor/paint.rs:26` (`pub(super)`). Private copy:
`chrome/hud.rs:12`. Same value (1.2), same role (text line height from font
size). Promote to a shared location (`render/text.rs` or `widgets/mod.rs`) and
import. Opportunistic — fold into any nearby touch.

### R11. Shared test-fixture crate (~200 test LOC, med risk) — REFACTOR, optional tier

27 hand-rolled `OutputInfo` builders across four crates (each 5-15 LOC):
capture-wayland ×12 (`kwin/tests.rs:39`, `icc/run.rs:279`,
`icc/protocol.rs:293`, `cursor/protocol.rs:266`, `portal/composite.rs:300`,
`portal/screencast/assemble.rs:125`, `portal/screenshot.rs:287`,
`portal/streams.rs:202`, `portal/run.rs:184`, `screencopy/run.rs:256`,
`screencopy/protocol.rs:272`, `stitch.rs:303`), ui ×14 (`launcher/tests.rs:20`,
`chrome/tests.rs:88`, `editor/magnifier/tests.rs:80`, `editor/tests.rs:355`,
`editor/tools/pixelate_tests.rs:87`, `editor/tools/tests.rs:77`,
`editor/outline.rs:101`, `selection/paint.rs:176`, `editor/paint.rs:293`,
`backdrop.rs:325`, `backdrop/scene.rs:161`, `backdrop/pixels.rs:205`,
`completion.rs:228`, `router.rs:317`), daemon ×1 (`tray/menu.rs:195`), plus
`tests/parity.rs:1240`, `tests/backdrop_composition.rs:116`. The canonical
fixture already exists: `MockBackend::default_outputs()`
(`capture/src/mock.rs:90-119`).

Also: `dual_layout()` in `launch/tests.rs:32` and `backdrop/scene.rs:175` are
structurally identical mixed-DPI dual-monitor layouts differing only in
connector names; `router.rs:323/334` adds `single_1080p`/`dual_mixed`;
`denial.rs` tests hand-build `FrameBuffer` literals ~6 times.

A `flowshot-testsupport` **dev-dependency** crate with `test_output(…)`,
`dual_mixed_layout()`, `single_1080p_layout()`, `test_frame(w, h, format)`
saves ~200 LOC net of its own code, with zero production-graph impact.

Risk (why this is the optional tier): many builders encode the exact
transform/scale/geometry **the test is about** — a shared fixture with
"sensible defaults" can silently decouple a test from its premise, and a
fixture change then ripples across four crates' suites. Migrate only the
generic builders; leave premise-carrying literals local. Domain stubs stay
local regardless (do-NOT-merge #7).

## REJECTED candidates (considered, not worth it)

- **icc↔screencopy `capture_run`/`outputs_run` skeleton** (~80 LOC): the
  connect→collect→select→per-output-capture→denial-check→assemble shape is
  copy-shaped, but the per-output capture call is the protocol-specific heart
  of each backend, and a generic `run_capture<E>(capture_one: impl Fn…)`
  closure would obscure exactly the part a reader needs to see per backend.
  Incidental duplication — the two one-shot flows will diverge with
  compositor quirks (that is what backends are for). The genuinely identical
  leaves (`select_targets`, `named_target`) are salvaged in R7.
- **`CaptureBackend` impl skeleton ×4** (~180 LOC): the trait impl shape
  (`kind`/`outputs`/`capture_outputs`/`capture_region`/`cursor_events`/
  `request_permission`) repeats across `icc.rs`, `screencopy.rs`, `kwin.rs`,
  `portal/*` — but permission semantics (denial frames vs portal response
  status vs name-owner checks) and cursor support genuinely differ per
  backend, and the trait is the cross-platform gate for the X11/Win/mac
  roadmap (draft D2/D5, plan todo 5). A generic skeleton helper would fight
  the next platform's impl, not help it.
- **Protocol state machines** (`icc/protocol.rs` 561, `screencopy/protocol.rs`
  616, `cursor/protocol.rs` 503 + the three `dispatch.rs`): different wire
  protocols with different event semantics (ICC multi-event constraint
  gathering + inverse remap; screencopy single buffer event + y-invert;
  cursor per-output sessions + coordinate-space conversion). Cannot be
  macro'd into a shared shape without inventing a protocol-DSL worse than the
  code. See do-NOT-merge #1.
- **Central timeout-vocabulary module** (~30 LOC): the 10s/15s/500ms
  constants (`icc/wait.rs:33`, `thread.rs:27,29`, `kwin/run.rs:38`,
  `portal/run.rs:40`, `cursor/run.rs:37`, `hyprland_ipc.rs:55`,
  `clipboard/keepalive.rs:24`) are each defined **exactly once at their use
  site** with good names. Repeated *values* across different roles is not
  duplication; a central `timeouts` module would be a junk drawer where a
  per-backend policy tweak looks like a global change. KEEP local.
- **Settings form-row "row builder"** (0 LOC saveable): the abstraction is
  already complete — `form.rs` owns `card`/`row`/`sub_section`/`hint`/
  `readout`, `fields.rs` owns `toggle`/`number`/`text_field`/`hex_color`/
  `combo`/`path_field`, and the tab files are thin composition (e.g.
  `general/upload.rs` is 48 lines of field calls). The `changed |= field(…)`
  repetition is idiomatic dirty-state accumulation, one distinct config
  binding per line. `interface.rs`'s `toolbar_list` (94-162) /
  `palette_editor` and `shortcuts.rs`'s `slot_row` recorder (150-198) are
  domain widgets, not form rows. Further abstraction would fight egui's
  closure borrows and obscure which field binds to which widget.
- **Event-routing unification** (0 LOC): the overlay already has ONE funnel
  (`state/route.rs`: chrome → editor → selection) and ONE coordinate mapper
  (`router.rs` `InputRouter`). Pins run in separate winit windows with
  window-local physical coords and compositor-delegated drag — a different
  machine entirely. Editor tool `bounding_rect()` impls are trait-required
  per-tool geometry, not duplication.
- **chrome/widgets re-implementation** (0 LOC): chrome delegates correctly —
  `toolbar.rs:199-203` uses `IconButton`, `side_panel/paint.rs:141,207,264`
  uses `Slider`/`Toggle`/`IconButton`. No shadow re-impl either: pins,
  toolbar, and panels all go through `DisplayList::shadow()` → the single SDF
  pass in `render/shadow.rs`.

## Oversized modules (250 pure-LOC production ceiling)

11 violations, measured production-only (inline tests excluded). Splits are
LOC-neutral (+mod/re-export lines) — the value is convention compliance and
navigability, not line count. `geometry.rs` and `scene.rs` are already
flagged in `docs/codebase-comparison.md:109-111` as "well past the size this
project would accept for new work".

| File | Prod LOC | Split suggestion |
|---|---|---|
| `core/src/geometry.rs` | 656 | `geometry/coord.rs` (PhysicalPx/Logical newtypes + Add/Sub + Coord/ToLogical/ToPhysical, L88-251), `geometry/shapes.rs` (Point/Size/Rect generics + aliases + impls, L253-614), `geometry/transform.rs` (Transform + TryFrom, L616-805), `geometry/layout.rs` (OutputInfo/OutputCrop/OutputLayout, L806-1005) |
| `core/src/scene.rs` | 600 | Rect/Ellipse/TextObject (L302-465) → existing `scene/objects.rs`; UndoStack (L812-940) → `scene/undo.rs`; Scene impl (L529-786) → `scene/graph.rs`; hub keeps Point/Rect/Color/PaintSink/ToolObject/ToolObjectData + re-exports (~250) |
| `core/src/config.rs` | 407 | `config/schema.rs` (section structs + Defaults, L75-471 — split `schema/tools.rs` L259-355 out if still over), `config/migrate.rs` (MIGRATIONS + migrate fns, L518-543), hub keeps Config struct + load/save + FromStr (L472-637) |
| `ui/src/render/renderer.rs` | 276 | Split the single `Renderer` impl (L80-276): pipeline construction vs frame execution/readback |
| `ui/src/render/tess.rs` | 275 | `tess/paths.rs` (shape_path/add_rect/add_rounded_rect/add_ellipse/box2d/lpoint, L153-308); hub keeps Tessellator + vertex constructors (~125) |
| `ui/src/chrome/state/input.rs` | 266 | `toolbar_action` + `edit_tool_config` (L271-320) → `chrome/state/toolbar_action.rs`; optionally split the ChromeState impl (L25-226) pointer vs keyboard |
| `ui/src/settings/model.rs` | 262 | Tab/ThemeChoice/Banner/FieldIssue/RecorderTarget + range consts (L26-131) → `settings/model/kinds.rs`; hub keeps SettingsModel + impl (~150) |
| `ui/src/editor.rs` | 262 | Hub is 33 mod decls + EditorState struct + one large impl (L156-EOF); move impl blocks into existing submodules (e.g. `editor/state.rs`) |
| `daemon/src/tray/dbusmenu.rs` | 259 | `layout_of`/`children_values`/`node_props` (L65-119) → `tray/dbusmenu/layout.rs`; hub keeps the two DbusMenu impls (~200) |
| `ui/src/egui_host/input.rs` | 257 | ClipboardBridge (L22-71) → `egui_host/clipboard.rs`; hub keeps InputState (~205) |
| `daemon/src/tray/item.rs` | 251 | Borderline: one cohesive zbus SNI interface; splitting props-getters vs action-methods harms cohesion more than the 1-LOC violation. Accept or shave |

Warning band (200-250, within convention, no action): 31 files, largest
`settings/form.rs` 243, `capture-wayland/session.rs` 242.

## Speculative-abstraction audit

Every trait in the workspace, implementer counts (production + test), verdict.
**Result: zero deletions.** The single-impl traits are exactly the three the
decision records mandate.

| Trait | Where | Prod impls | Test impls | Verdict |
|---|---|---|---|---|
| `CaptureBackend` | capture/backend.rs:100 | 6 (icc, screencopy, kwin, portal×2, mock) | — | KEEP — the architecture + cross-platform gate (D2/D5) |
| `Tool` | ui/editor/tool.rs:156 | 14 | 10 | KEEP |
| `ToolObject`(+Clone) | core/scene.rs:231 | 9 | — | KEEP |
| `PaintSink` | core/scene.rs:165 | **1** (ListSink) | 3 | KEEP — crate-purity seam: core must paint without depending on ui's DisplayList (plan todo 4 "renderer-agnostic sink trait"; purity gate todo 42) |
| `PinActionSink` | ui/pins/sink.rs:52 | **1** (daemon ActionBridge) | 1 (example) | KEEP — dependency inversion: ui-lib may not depend on daemon/actions (`ui/Cargo.toml:57-60` dev-dep comment); the bridge lands in the binary layer |
| `Uploader` | actions/upload/mod.rs:47 | **1** (Imgur) | — | KEEP — user-mandated plugin point: "Imgur behind a pluggable provider trait" (plan Scope; draft C4) |
| `ClipboardBackend` | actions/clipboard.rs:64 | 1 + 1 mock | — | KEEP — test seam |
| `Clock` | daemon/lifecycle.rs:90 | 1 (Tokio) + 1 MockClock | — | KEEP — deterministic lifecycle tests |
| `CommandSink` | daemon/command.rs:49 | 5 | — | KEEP |
| `NotifySink` | actions/export/mod.rs:28 | 5 | — | KEEP |
| `FileDialogSink` | actions/export/mod.rs:43 | 5 | — | KEEP |
| `Notifier` | daemon/notify.rs:46 | 3 | — | KEEP |
| `UriOpener` | daemon/notify/desktop.rs:31 | 1 | 2 | KEEP — test seam |
| `OutputProbe` | daemon/tray/outputs.rs:15 | 1 | 2 | KEEP — test seam |
| `Coord`/`ToLogical`/`ToPhysical` | core/geometry.rs | 2/4/4 | — | KEEP — load-bearing for the physical-first rule (#4871) |
| `BackendError` | capture-wayland/error.rs:304 | 5 | — | KEEP — enables the `_with::<E>` sharing that R3/R4/R7 extend |

`MonitorProbe`/`LaunchCallback` (launcher/options.rs:23,48) are concrete
function-holder structs, not traits — no seam inflation there. Generics audit:
the `_with::<E>` family has 4+ instantiations; no single-instantiation
generics found worth collapsing.

## Deliberate duplication — do NOT merge

Recorded so future refactors don't relitigate these. Each looks like
duplication and isn't.

1. **Per-backend protocol state machines and dispatch impls**
   (`icc/protocol.rs`+`dispatch.rs`, `screencopy/…`, `cursor/…`, and the
   per-backend `assemble_frame`s). Different wire protocols with different
   event semantics; each backend is a per-compositor quirk domain pinned by
   the stub-contract-fidelity rule (Verification strategy, plan: stubs pin
   upstream URL+commit) and plan todo 11's MUST-NOT ("no KDE-only code paths
   leaking into other backends"). They are *expected* to diverge.
2. **The `CaptureBackend` impl skeleton per backend** (~60 LOC × 4). The seam
   is the architecture (draft D3 ladder, D5 crate topology); permission
   semantics and cursor support differ per rung, and the same trait must
   accept future X11/WGC/ScreenCaptureKit backends (F19/F20) whose skeletons
   will look different again.
3. **Per-crate typed error enums.** Amendment #4: thiserror in all lib crates,
   anyhow only at binary edges, no stringly errors across crate boundaries.
   The From-chains between crates are the typed-boundary tax, paid
   deliberately. (R1 merges *within* capture-wayland's backend family only.)
4. **`scene::Point/Rect/Color` vs `geometry::Point/Rect`.** scene's are f32,
   serde-serialized persistence types in editor-local space
   (`core/scene.rs:50-135` — the on-disk/undo-snapshot format depends on
   their exact shape); geometry's are scale-safe typed-newtype screen algebra
   (`geometry.rs:253+`, half-open contains vs scene's closed). The
   physical-pixels-first rule (#4871, draft F27) exists precisely to keep
   these spaces from conflating. The ~30 LOC of op-surface overlap is the
   price of the boundary.
5. **Per-domain timeout/policy constants.** Defined once per site with role
   names (`CAPTURE_TIMEOUT` vs `KWIN_TIMEOUT` vs `CURSOR_POS_TIMEOUT`); equal
   values are coincidence, not shared policy. No central module (see REJECTED).
6. **Per-surface `strings.rs` modules** (cli, daemon, settings, launcher,
   pins). The i18n-ready message-catalog architecture (Amendment #4
   conventions) wants per-surface catalogs; merging them creates one
   god-module and breaks the future extraction story.
7. **Domain-specific test stubs.** `kwin/stub.rs` (pins the fetched KWin
   source URL+commit per the stub-contract-fidelity rule),
   `daemon/src/testsupport.rs` (D-Bus harness), the portal-shortcut stubs.
   Their specificity IS the contract; a generic stub factory would hide the
   pinned upstream behavior. (R11 covers only generic geometry/frame
   builders.)
8. **Overlay and pins window lifecycles vs the egui shells.** Overlay =
   per-monitor fullscreen transparent multi-window with frozen backdrop (D1,
   D3: capture-before-overlay); pins = long-lived multi-window
   custom-rendered with compositor-delegated drag; settings/launcher =
   single opaque egui dialogs (D8b exception). Only the last two share a
   lifecycle (R2). Merging pins/overlay into any shared "window shell" would
   force one event model onto three genuinely different ones.
9. **Two widget systems: egui (settings/launcher) vs the token-driven custom
   micro-widget layer (`widgets/`, overlay/pins/editor).** D8 records this as
   the accepted tradeoff of the egui-settings exception ("full widget kit,
   themeable, no second windowing stack" — the stack is shared, the widget
   kits are not). Unifying them was already rejected at plan review; do not
   relitigate via a "dedup" refactor.
10. **`icc/shm.rs` and `icc/wait.rs` live under `icc/` but are crate-shared**
    via `_with::<E>` generics (screencopy, kwin, portal, cursor all call
    them). The path is historical; the sharing is real. Do not "fix" this by
    re-duplicating per backend, and do not read the `icc/` path as ICC-only.
    (A cosmetic rename to `shm.rs`/`wait.rs` at crate root is fine but saves
    nothing.)

## Honest floor

- Production code: 38,254 pure LOC. Realistic dedup delta: **~709 LOC
  (1.9%)**, of which ~450 (R1+R2) is two focused refactors and ~260 is nine
  small mechanical merges. There is no thousands-of-lines win hiding here —
  the shared layers (capture infra, render/, widgets/, form.rs, the route
  funnel) were built during initial execution, not retrofitted.
- Tests + QA harnesses: 31,470 pure LOC. Per the project's own strategy (D4
  "test the core, QA the edges"; 1,218 green tests; examples are the live-QA
  harnesses), these mostly stay. R11's ~200 LOC is the only defensible test
  delta, and it is optional.
- Ceiling compliance: 11 files to split, LOC-neutral. If the goal is "less
  code", the splits don't help the number; if the goal is "kept clean", they
  are the highest-value item in this report after R1/R2.
- Suggested execution order if pursued: R1, R2 (each a focused PR), then
  R3+R4+R7+R9 as one capture-wayland consolidation PR, R5+R8+R10 as one
  ui/core consolidation PR, R6 standalone (release-contract surface), R11
  last and only if the fixture drift risk is accepted.
