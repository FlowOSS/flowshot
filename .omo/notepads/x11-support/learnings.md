# Learnings — x11-support

## 2026-10-04 Exploration synthesis (5 explore agents)

- `BackendKind::X11` exists as roadmap variant; promotion = `is_roadmap()` (kind.rs:64-66),
  `protocol_name()` (kind.rs:84-95), `NEGOTIATION_LADDER` (negotiate.rs:46-52), daemon
  `construct()` arm (execute/backend.rs:102-117), plus test-matrix updates.
- `CapabilityProbe::supports()` hard-filters roadmap kinds — removing X11 from
  `is_roadmap()` is sufficient, no `supports()` change needed.
- Clipboard: `ClipboardBackend` trait ALREADY exists (actions/src/clipboard.rs:64-72);
  X11 = sibling impl + `Clipboard::for_session()`. All 6 call sites use `Clipboard::wayland()`.
- Wayland crate blueprint: worker.rs (spawn_worker bridge), stitch.rs (OutputLayout),
  icc/shm.rs (memfd + file-I/O readback — mirror with memfd + shm::attach_fd on X11,
  zero unsafe, no SysV shmat).
- Purity gate only audits flowshot-core/flowshot-capture/flowshot-ui — new platform
  crate is exempt automatically. deny.toml allows MIT/Apache-2.0 → x11rb passes.
- UI gate `require_display_server` (ui/src/runtime.rs:215) checks WAYLAND_DISPLAY only;
  Phase A keeps UI Wayland-gated but must REWORD the error message (claims "never falls
  back to X11" — becomes false).
- Daemon probe path is Wayland-specific (CaptureThread::spawn at execute/backend.rs:58);
  needs a session-type branch (decision #7 in the plan).
- x11rb 0.14 API notes: `x11rb::connect(None)`, RANDR `get_monitors`, XFIXES
  `get_cursor_image` (position+pixels), SHM `query_version`/`attach_fd`/`get_image`,
  Xft.dpi from root `RESOURCE_MANAGER` property. INCR protocol needed for large
  clipboard payloads (> ~256KB request-size limit).

## [2026-10-04] Task 1: promote BackendKind::X11

**Result**: DONE. `cargo test -p flowshot-capture` = 35 lib + 15 doc tests green.
`./scripts/purity-gate.sh` = PASS (0 violations, 2 pre-existing allowlisted hits, NO
new allowlist entries). `cargo build -p flowshot-daemon` = compiles. Files touched:
`crates/flowshot-capture/src/{kind.rs,negotiate.rs}` ONLY.

### Purity gate: did NOT trip — the L29 comment's claim holds, and is understated

Read the exact regexes before editing. `banned_ids_alt` carries `x11[a-z_0-9]*` and
`xcb`, but both only fire in **import-shaped** positions:
- `src_patterns[0]` needs `use`/`extern crate` before the identifier.
- `src_patterns[1]` needs `::` immediately after it.
- `grep -E` is **case-sensitive**, so `BackendKind::X11` (capital X) never matches `x11…`.
- `#[serde(rename = "x11")]` is followed by `"`, not `::` → invisible.
- NEW third invisible form my change adds: the diagnostic string literal
  `"X11 (xcb GetImage)"` sits on a *code* line (not a stripped `//` comment) and
  contains lowercase `xcb` — still invisible, because `xcb` is followed by a space,
  not `::`, and there is no `use`. Verified empirically: gate PASS.

**No gate change was needed.** Optional touch-up for task 7/9: the L29 comment lists
only two invisible forms (`BackendKind::X11`, serde rename `"x11"`); a diagnostic
*string literal* naming a banned platform crate (`xcb`) is a third. Comment-only,
zero behavioral risk — do NOT widen the regex to catch it, that would false-positive
on every legitimate diagnostic message.

### The single switch, confirmed

`CapabilityProbe::supports()` (negotiate.rs L101) was left untouched per scope:
`!kind.is_roadmap() && self.available.contains(&kind)`. Dropping `Self::X11` from
`is_roadmap()`'s `matches!()` is sufficient — X11 became negotiable with no
`supports()` edit, exactly as the inherited wisdom predicted.

### Tests: 4 forced regressions prove the new guards can fail

Temporarily reverted each change, watched the named tests FAIL, restored green:
- **A** X11 back in `is_roadmap()` → 3 fail (`x11_only_probe_negotiates_the_x11_rung`
  on `assert!(x11.supports(X11))`, `force_x11_on_an_x11_probe_selects_it`,
  `full_probe_preserves_exact_ladder_order`).
- **B** X11 dropped from `NEGOTIATION_LADDER` → 3 fail (incl.
  `empty_probe_error_names_every_missing_protocol` via the new `missing.len() == 6`).
- **C** `protocol_name()` X11 arm reverted → 2 fail (both pin the literal
  `"X11 (xcb GetImage)"`).
- **D** "X11 always available" bug injected into `supports()` → 11 fail, incl.
  `wayland_probe_does_not_gain_the_x11_rung` (its designated regression).

Anti-tautology fix worth copying: `full_probe_preserves_exact_ladder_order` previously
asserted `backends == NEGOTIATION_LADDER.to_vec()` — **expected derived from the same
const as the output**, so dropping X11 from the ladder shrank both sides and it stayed
green. Rewritten to an explicit 6-element literal vec. Same reason `missing.len() == 6`
and the literal protocol-name string were added to the empty-probe test: they are the
only non-circular pins on ladder arity.

### Doc debt my change created (fixed in-scope) and did NOT (out-of-scope, follow up)

Fixed (both in allowed files, both falsified by the promotion — leaving them would
ship docs that lie about the contract):
- `kind.rs` enum rustdoc: "first **five** variants form the ladder" → six; "last
  **three** variants are roadmap placeholders … `supports` reports `false` for them"
  → last **two**, non-Linux. Plus the X11 variant doc and the `is_roadmap` doctest.
- `negotiate.rs`: ladder rustdoc (task item #4) and `negotiate()`'s explicit
  rung-chain enumeration (gained `-> X11`).

**STILL STALE — not mine to touch, assign to task 5/9:**
- `crates/flowshot-capture/src/lib.rs` L13-16: crate-level doc still enumerates the
  ladder as 5 rungs (`ext-image-copy-capture -> wlr-screencopy -> KWin ScreenShot2
  -> portal ScreenCast -> portal Screenshot`), no X11.
- `crates/flowshot-capture/src/backend.rs` L4: "the roadmap `X11`/Windows/`macOS`
  backends" — X11 is no longer roadmap.
- `crates/flowshot-ui/src/{runtime.rs L92,L212-214; error.rs L15,L20}`: "never falls
  back to X11" — already task 6's job, confirmed still present.
- `docs/porting-roadmap.md` L35/L61 describe X11 via `is_roadmap()`/ladder-as-data —
  task 9.

### LOC / size note (measured, justified exception — NOT refactored)

`negotiate.rs` pure LOC: 242 @HEAD → **293** now, i.e. over the 250 ceiling.
Breakdown: **lib portion = 74 pure LOC**, `#[cfg(test)] mod tests` = 219. All growth
is the mandated test coverage (4 new tests + strengthened assertions); production
logic gained 1 array element and 1 type-level `5`→`6`. The file still owns exactly one
thing ("capability probing and ladder negotiation" — no "and"), and inline `mod tests`
is this crate's uniform convention (`mock.rs` = 414, `error.rs` = 78, `kind.rs` = 58).
Splitting would mean creating files outside my allowed set mid-plan while a sibling
task runs concurrently. **Recommended follow-up if it grows again**: extract to
`negotiate_tests.rs` — the repo already has that precedent at
`crates/flowshot-ui/src/editor/pixelate_tests.rs`.

### Concurrency hazard for the orchestrator

The working tree already carried **uncommitted changes from a sibling task** when I
started: `Cargo.lock` (+`x11rb 0.14.0`, `windows 0.61.3`→`0.62.2`) and
`crates/flowshot-actions/Cargo.toml` (`+x11rb = "0.14"`) — that is task 4 (X11
clipboard) in flight. I did not touch either. Do not attribute them to task 1, and do
not `git checkout`/`stash` the tree while tasks overlap.

## [2026-10-04] Task 6: UI gate message
- Updated UiError::NoDisplayServer error message to reflect that FlowShot is no longer Wayland-native and never falls back to X11
- Message now explains that interactive UI requires Wayland but headless capture works on X11
- Preserved existing error variant and behavior (require_display_server() logic unchanged)

## [2026-10-04] Task 4 (resumed): X11 clipboard

**Result**: DONE. build/test (99 passed, 13 of them x11 unit tests)/clippy
`-p flowshot-actions --all-targets -D warnings`/fmt ALL GREEN. purity-gate PASS
(0 violations, same 2 pre-existing allowlisted hits as task 1 — no drift).
`cargo deny check` clean (advisories/bans/licenses/sources ok; x11rb 0.14.0
MIT/Apache-2.0). LIVE smoke on i3 DISPLAY=:0 PASSED twice (reproducible).

### Audit verdict on the interrupted partial work: design CORRECT, did not compile

The 830-line `clipboard/x11.rs` partial is architecturally sound — every spec
item verified present and correct by line-by-line audit + x11rb 0.14 registry-
source API verification (no guessing):

- **Timestamp trick** (ICCCM 2.1): zero-length `ChangeProperty(APPEND)` on a
  private atom `FLOWSHOT_CLIPBOARD_TS` on the 1x1 InputOnly owner window
  (PROPERTY_CHANGE selected at create) → server emits `PropertyNotify(NEW_VALUE)`
  carrying a valid server timestamp; 5 s deadline, 1 ms poll. `CurrentTime`
  never used for `SetSelectionOwner`. Ownership confirmed via
  `GetSelectionOwner == window` round-trip.
- **INCR threshold**: `chunk = max_request_bytes().saturating_sub(1024)
  .clamp(1, 256 KiB)` — `maximum_request_bytes` is on `RequestConnection`
  (BIG-REQUESTS-aware). On this Xorg (bigreq, 16 MiB max) chunk = 256 KiB cap.
- **Sender-side INCR** (the flagged hard part) traced correct: select
  PROPERTY_CHANGE on requestor → property type INCR + u32 total →
  one `change_property8` chunk per `PropertyNotify(DELETE)` keyed by
  (requestor, property) → zero-length REPLACE terminator → entry dropped.
  Own NEW_VALUE notifies fall through the `_` arm harmlessly. Re-request
  supersedes via HashMap insert. 1 MiB live round-trip byte-identical.
- **MULTIPLE** (ICCCM 2.6.2): ATOM_PAIR list read (4096 4-byte-unit cap),
  per-pair conversion, failed/unknown/nested-MULTIPLE pairs → `DeleteProperty`
  (type None), unreadable list → refuse (notify property None).
- **Supersede**: serve() blocks only until ownership reported (mpsc ready
  channel), replaces the `Serving` record, detaches the old JoinHandle; the
  server's `SelectionClear` exits the old thread quietly. No thread leak.
- Own `RustConnection` created ON the serving thread. Zero unwrap/expect,
  no unsafe.

**What the interruption left broken (all fixed, minimal diffs):**
1. Workspace would not even LOAD: task 2's `flowshot-capture-x11` was
   member-declared with `src/error.rs` but no `src/lib.rs`. Created a minimal
   scaffold lib.rs (`#![forbid(unsafe_code)]` + `pub mod error;` + SCAFFOLD
   NOTE) — additive, nothing reverted, task 2 extends it. error.rs compiles
   + its 6 tests pass.
2. x11.rs 4 compile errors: `Event` is `x11rb::protocol::Event` (NOT
   `xproto::Event`); `maximum_request_bytes` needs `RequestConnection` in
   scope; `acquire_timestamp<C: ConnectionExt>` → `<C: Connection>`
   (x11rb 0.14: `xproto::ConnectionExt: RequestConnection` only — flush/
   poll_for_event live on `Connection`; blanket `impl<C: RequestConnection>
   ConnectionExt` means a `Connection` bound suffices for both).
3. Test-code errors: `assert_eq!(Result<SessionKind, ClipboardError>, ..)`
   impossible (boxed dyn Error ⇒ no PartialEq) → `matches!`; `incr_walk`
   partial-move (`step @ (range, ..)` pushed then `range.end`) → clone range;
   error.rs test used `source` after `From` moved it → capture message first.
4. Clippy rust-1.99 sweep: `chunks_exact(8/4)` → `as_chunks::<8/4>()` (lib +
   example; atom destructuring now infallible), `needless_pass_by_value` on
   run_owner's Sender → `#[expect]` (ownership IS the design: the drop closes
   the channel and unblocks serve()'s thread-exited error path), doc_markdown
   `XWayland`, map_identity, single_match_else in the example.
5. **6 PRE-EXISTING `assert_is_empty`/`assert_not_empty` failures in actions**
   (same toolchain-regression class task 1 recorded for capture/mock.rs:419):
   backend.rs:240, pipeline.rs:447+504, history.rs:151 → `assert_eq!(x, [] as
   [T; 0])`; encode.rs:96-97 `assert!(!x.is_empty())` DELETED — strictly
   implied by the `&x[..3]` JPEG-magic asserts two lines above (empty vec
   panics on the slice), so they were redundant verification, not coverage.
   My gate is crate-wide clippy clean ⇒ in-scope. capture/mock.rs:419 remains
   open for task 7 (not my crate).

### Live smoke evidence (i3, DISPLAY=:0, run twice, both exit 0)

```
x11 smoke OK: direct text, TARGETS (8 targets), supersede, 1024 KiB INCR round-trip
```
- Small text offer served, read back over a SECOND independent x11rb
  connection via real ConvertSelection/SelectionNotify — byte-identical
  ("flowshot x11 smoke" via UTF8_STRING target).
- TARGETS = 8 atoms: TARGETS, TIMESTAMP, MULTIPLE, text/plain,
  text/plain;charset=utf-8, UTF8_STRING, STRING, TEXT — all required present.
- Supersede: second serve() replaced the first; read-back from the new offer.
- INCR: 1 MiB payload (4x 256 KiB chunks + terminator) round-tripped
  byte-identical (length + all-bytes asserted).
- `xclip`/`xsel` NOT installed on this machine (recorded in issues.md); the
  second-x11rb-client read-back is the spec-sanctioned fallback and exercises
  the identical wire protocol xclip would.

### Size verdict (measured)

- `clipboard/x11.rs`: 656 pure LOC (≈540 lib + ≈116 tests) — over the 250
  ceiling. Marked `// allow: SIZE_OK`: one indivisible ICCCM protocol state
  machine (ownership/timestamp/dispatch/INCR/MULTIPLE share one OwnerState);
  splitting mid-plan (task 5 consumes X11Clipboard next) scatters the protocol
  flow for zero behavioral gain. Same judgment class as task 1's negotiate.rs
  (293). Follow-up if it grows: extract `x11/pure.rs` (targets_for/incr math/
  atom_pairs + their tests, ≈230 lines) first.
- `clipboard.rs`: 245 pure LOC — WARNING BAND (200-250). Task 5 switches call
  sites in the daemon, not here; if any edit adds lines to clipboard.rs,
  propose a split (routing/session detection is the natural seam).
- error.rs 78, x11_smoke.rs 186, capture-x11 lib.rs 10 — healthy.

### x11rb 0.14 API facts verified against ~/.cargo/registry (for tasks 2/3)

- `x11rb::protocol::Event` (re-export of x11rb_protocol) — there is NO
  `xproto::Event`. Variants: `SelectionRequest/SelectionClear/
  SelectionNotify/PropertyNotify(xproto::…Event)`, `Error(X11Error)`.
- Trait split: `RequestConnection` (generate_id? NO — see next) vs
  `Connection` (wait_for_event, poll_for_event, flush, generate_id L424) vs
  `xproto::ConnectionExt: RequestConnection` (all core requests; blanket impl
  for every `C: RequestConnection`). `maximum_request_bytes` is on
  RequestConnection (L349) — import it explicitly.
- `send_event(propagate, dest: impl Into<Window>, mask, event: impl
  Into<[u8; 32]>)`; `From<SelectionNotifyEvent> for [u8; 32]` exists.
- `SelectionNotifyEvent` fields: response_type/sequence/time/requestor/
  selection/target/property. Const `SELECTION_NOTIFY_EVENT: u8 = 31`.
- `Property::NEW_VALUE`/`Property::DELETE`, `EventMask::NO_EVENT`,
  `x11rb::{NONE, COPY_FROM_PARENT, CURRENT_TIME}` all confirmed.
- `get_property(delete, window, property: impl Into<Atom>, type_: impl
  Into<Atom>, long_offset: u32, long_length: u32)`; format-32 values are
  native-endian u32 bytes in `.value` (to_ne_bytes/from_ne_bytes correct).
- New clippy (rust-1.99): `chunks_exact_to_as_chunks` fires on constant chunk
  sizes — use `slice::as_chunks::<N>()` (stable 1.88+).

## [2026-10-04] Task 10: unique_tempdir hardening

Applied a process-wide AtomicU64 counter to the unique_tempdir() function in three locations:
- crates/flowshot-actions/src/clipboard/pipeline.rs:327
- crates/flowshot-actions/src/export/path.rs:106
- crates/flowshot-actions/src/upload/history.rs:124

Mechanism: Added static AtomicU64 counter that increments with each call to ensure unique directory names even under high-concurrency loads.

Test results: 20 consecutive runs of 'cargo test -p flowshot-actions' all passed, confirming the fix resolves the collision issue under full-workspace load.

## [2026-10-04] Task 3: x11 capture path

**Result**: DONE. `cargo test -p flowshot-capture-x11` = 62 lib + 5 doc tests green
(task 2's 33 preserved + 34 new headless: format/stride mapping, transform inverse-remap
round trip, region-crop stitch math, cursor ARGB->RGBA/hotspot/composite clipping,
worker bridge, backend contract). `cargo clippy -p flowshot-capture-x11 --all-targets
-- -D warnings` CLEAN (incl. task 2's tail: 2 redundant-closure in probe.rs ->
`Cookie::reply`, 2 doc_markdown `HiDPI` in scale.rs). `cargo fmt --check` CLEAN
(workspace-wide). purity-gate PASS (0 violations, same 2 pre-existing allowlisted hits).
`cargo deny check` ALL OK. `cargo build --workspace` green. LIVE probe + capture +
pixel-oracle + cursor-paint all verified on i3 DISPLAY=:0 (evidence below).

### Files (all inside flowshot-capture-x11; no forbidden crate touched)

- NEW `src/capture.rs` (lib 203): capture_run, validate_pixel_format (depth-24 +
  LSBFirst gate), negotiate_shm, grab_screen/grab_shm/grab_plain, assemble_frame
  (pure, headless-tested), shm_fast_path.
- NEW `src/cursor.rs` (lib ~170): cursor_pos (pub, QueryPointer one-shot),
  physical_to_global (pure), cursor_snapshot (XFIXES, per-connection negotiation),
  XrgbCanvas + composite_cursor_xrgb + blend_over_opaque (adapted from wayland
  cursor/protocol.rs composite_cursor_rgba for opaque XRGB destinations).
- NEW `src/stitch.rs` (lib 190 + tests 279): CapturedOutputs::stitch + to_rgba (pub,
  line-by-line adaptation of wayland stitch.rs; duplication deliberate - sibling
  platform crates must not depend on each other; follow-up for task 9: consider
  hoisting into flowshot-capture as shared platform-free algebra).
- NEW `src/worker.rs` (lib 40): spawn_worker, wayland pattern pinned to X11Error.
- NEW `src/backend.rs` (lib 77): X11Backend {caps, display} + CaptureBackend impl
  (kind=X11, cursor_events=None, request_permission=NotRequired, region never paints
  cursor) + `cursor_pos()` async + `without_shm()` diagnostic builder.
- NEW `examples/probe.rs`, `examples/capture.rs` (both run LIVE, output below).
- MOD `src/error.rs`: +Io(#[from] io::Error), +UnsupportedByteOrder,
  +ImageSizeMismatch{expected,actual} (additive; existing From->CaptureError covers).
- MOD `src/output.rs`: +MonitoredOutput{data,info} + enumerate() (pairs screen-space
  MonitorData with OutputInfo so capture never round-trips rects through f64 logical);
  outputs() delegates; zero-extent monitors skipped+warned (GetImage would BadValue).
- MOD `src/lib.rs` (re-exports: X11Backend, cursor_pos, pub mod stitch), `Cargo.toml`
  (+nix, +futures - both pre-existing workspace deps; NO new workspace dependency,
  NO workspace feature change).

### SHM VERSION FINDING - the plan/task text "SHM >= 1.15" is WRONG; fd-passing is 1.2

There is no MIT-SHM 1.15. AttachFd/CreateSegment (fd-passing) landed in MIT-SHM **1.2**
(Xorg 1.19, 2016); x11rb's own examples/shared_memory.rs gates fd-passing on 1.2
("Check for SHM 1.2 support (needed for fd passing)"). This Xorg reports **shm 1.2**
live - gating on >=1.15 as the task text said would have made the fast path DEAD CODE
EVERYWHERE. Implemented gate: `at_least(1, 2)` (SHM_FD_PASSING_MAJOR/MINOR consts in
capture.rs, headless-tested). **Deviation from task wording, forced by protocol reality
- task 9 docs should record 1.2, and the plan's decision #3 text ("server SHM >= 1.15")
should be corrected.**

### Pixel-format oracle result (LIVE, required verification)

xsetroot NOT installed (issues.md) -> oracle = solid-red window: `magick -size 640x480
xc:'#FF0000'` shown via ImageMagick `display` on a fresh i3 workspace (static screen,
zero churn), captured through examples/capture, cross-checked against ImageMagick
`import -window root` (an independent XGetImage reader):
- Ours at 3 in-window points: srgba(255,0,0,1); import: srgb(255,0,0) - IDENTICAL.
  Gray window-bg point (214,214,214) identical too. Control crop AE=0 (pixel-exact).
- Raw memory byte order for red = B,G,R,X = [0,0,255,X] -> **FrameFormat::Xrgb8888
  mapping CONFIRMED** (little-endian 0x00RRGGBB = wl_shm XRGB8888 semantics).
- X padding byte on this Xorg = **255** (not 0) - harmless: X is unused, to_rgba
  forces alpha 255.
- Full-frame live geometry: 2880x1620, stride 11520 (= width*4 tight), 18,662,400
  bytes = w*h*4 exact. Region 800x600 logical -> 800x600 RGBA8888 composite scale 1.0.
- Whole-screen live cross-check (dynamic screen): compare -metric AE vs import =
  166.9 px of 4.66M (0.0036%) - all screen churn between grabs.

### Per-output timing GetImage vs SHM (measured live, 2880x1620 = 18.6 MB/frame)

shm-capable pass **26-31 ms** vs forced-plain pass (without_shm()) **72-89 ms** ->
fd-passing fast path is **~2.7-3x faster** on this machine. (examples/capture prints
both; per-output elapsed_ms + path also logged at debug from grab_screen.)

### XFIXES per-connection negotiation gotcha (cost me a silent degradation - FIXED)

Xorg's ProcXFixesDispatch (Xext/xfixes/xfixes.c) rejects EVERY request before
per-connection QueryVersion (`version_requests[major_version=0]` allows only
QueryVersion -> GetCursorImage = BadRequest). Each capture worker holds a FRESH
connection - the construction-time query_caps negotiated a DIFFERENT one - so
GetCursorImage silently degraded (cursor painting off, debug-level only). Fix:
cursor_snapshot runs xfixes::query_version(5,0) on the worker connection first.
MIT-SHM: Xorg's ProcShmDispatch does NOT enforce QueryVersion before AttachFd
(live-confirmed: fast path worked pre-negotiation), but the spec wants it -> capture_run
negotiates shm once per run too (1 round-trip, other servers may enforce). RANDR:
no enforcement issue (task 2's outputs() already worked on fresh connections live).
**Rule for task 5+: any X extension used on a one-shot connection must negotiate its
version ON that connection.**

### Cursor facts (live-verified)

- GetCursorImage reply x,y = HOTSPOT position on screen (Xorg GetSpritePosition;
  OBS/GStreamer/Weylus all compute render pos = (x-xhot, y-yhot)); top_left =
  (x - xhot, y - yhot). Pixels = CARD32 0xAARRGGBB -> converted to RGBA8888.
- Cursor painting LIVE-VERIFIED post-fix: static red screen, painted-vs-unpainted pass
  diff = 488 px with blob at pointer (1788,287); isolated 64x64 crop at pointer:
  ours-vs-import AE=116 (the painted glyph; import excludes the hardware cursor),
  control crop AE=0; zoom shows 248 distinct colors (anti-aliased glyph).
- **GetImage on root EXCLUDES the hardware cursor** on this machine (modesetting/ddx
  hardware cursor) - so paint_cursor composites exactly once. KNOWN LIMITATION: on
  drivers using SOFTWARE cursors the framebuffer already contains the cursor and
  paint_cursor would double-paint; no X11 API asks the server to exclude it. Recorded
  in issues.md for task 8 QA (verify on other hardware if it ever appears).
- Cursor is composited in SCREEN space BEFORE the inverse-transform remap, so it
  rotates/flips with the output content (correct for transformed outputs).
- cursor_pos(): QueryPointer root_x/root_y -> containing monitor -> logical origin +
  local/scale. same_screen=false -> None. Live: (794.67, 127.56) logical =
  physical (1788,287)/2.25 on eDP-1.

### Mixed-scale logical layout property (task 2 semantics, pinned by new tests)

output.rs derives logical_rect = screen_rect / OWN scale (task 2, reviewed). On a
mixed-scale layout this leaves logical GAPS between outputs (eDP-1 s2.25 spans logical
[0,1280); a scale-1 DP-1 at physical x=2880 starts at logical 2880, not 1280). The
cursor/region math stays self-consistent PER OUTPUT (physical_to_global + crop algebra
both use the output's own rect+scale), but the logical layout is not a compacted
representation of the physical one. Two of my cursor tests initially assumed Wayland-
style compacted origins and FAILED - production code was right, fixtures corrected to
pin the actual contract. Single-output (this machine) and same-scale layouts unaffected.

### Live probe output (this machine, for the record)

```
DISPLAY: ":0" | randr 1.5 | xfixes 5.0 | shm 1.2 (fd-passing: true)
root: screen 0 2880x1620, depth 24, LSBFirst
eDP-1: logical (0,0 1280x720) physical 2880x1620 scale 2.25 transform Normal
```
Xft.dpi ABSENT on this machine (xprop -root RESOURCE_MANAGER has no Xft.dpi) ->
scale 2.25 comes from the mm heuristic (2880px/344mm ~ 212.7dpi ~ 2.215 -> 2.25).

### Deps / size / review notes

- nix 0.29 gates `sys::memfd` behind feature **"fs"** - already in the workspace nix
  features ("fs","poll"). **No "memfd" feature exists/needed; no workspace change.**
- Crate manifest gained `nix` + `futures` (both pre-existing workspace deps; futures
  is required by the spawn_worker bridge and already exposed through flowshot-capture's
  CursorStream). Cargo.lock: only this crate's dependency list changed.
- LOC (pure): capture.rs 203 lib (WARNING BAND 200-250 - next edit adding lines should
  split transport (grab_*) from assembly (assemble_frame/validate)); output.rs 206 lib
  (WARNING BAND, was ~180 at task 2, enumerate refactor added the pairing); cursor.rs
  ~170, stitch.rs 190, worker.rs 40, backend.rs 77, examples 64/112 - healthy. Totals
  >250 in capture/cursor/stitch/output are inline #[cfg(test)] mods (uniform repo
  convention, cf. task 1's negotiate.rs note).
- blit_crop keeps the wayland blueprint's 5-param shape (source, physical crop,
  destination rect, dest data, dest width): line-by-line adaptation of reviewed sibling
  code, each input a distinct geometric role; grouping would diverge from the blueprint
  F1 reviewers diff against. composite_cursor_xrgb DID get the canvas treatment
  (XrgbCanvas, 2 params) matching wayland's RgbaCanvas.
- For task 5 wiring: `X11Backend::connect() -> Result<Self, X11Error>` (probe caps at
  construction), `Box<dyn CaptureBackend>` ready (trait-object test pins it),
  `backend.cursor_pos().await -> Result<Option<LogicalPoint>, CaptureError>` parallels
  resolve_cursor_pos, `X11Backend::display()`/`caps()` for diagnostics/telemetry.

## [2026-10-04] Task 5 (resumed): daemon wiring complete

**Result**: DONE. `cargo build --workspace` green. `cargo test --workspace`: 38 suites ok,
2 pre-existing environmental failures (ui CJK fonts — issues.md; cli parity_matrix
missing `.omo/evidence/` — NEW finding, issues.md). daemon lib 131/131 after fixing the
stale X11 guard in telemetry/environment.rs (in-scope sibling of the payload.rs:220 fix).
`cargo clippy --workspace --all-targets -- -D warnings`: 45 pre-existing rust-1.99
assert_is_empty/assert_not_empty hits (task 7's sweep, issues.md list refreshed) — ZERO
in task-5 files (location set byte-identical to pre-task). `cargo fmt --check` clean.
purity-gate PASS (0 violations, same 2 allowlisted hits). CLI needs NO flowshot-capture-x11
dep (exit.rs downcasts IccError only; X11 has no permission error) — plan text said both
Cargo.tomls, deviation deliberate (no unused deps).

### Files touched (all daemon + 1 ui example, per scope)
- `execute/backend.rs`: session routing via `flowshot_actions::clipboard::detect_session()`
  (REUSED, not re-implemented: one tested implementation of decision #7 shared by capture
  routing + clipboard pick; WAYLAND_DISPLAY wins, empty = unset). X11 leg `open_x11_session`
  (probe_x11 → None = warn + typed NoBackendAvailable{missing:[X11]}; negotiate → [X11];
  ladder walk MIRRORED not shared — Wayland leg untouched per mandate, stitch.rs
  sibling-duplication precedent). `construct()` now `Result<Box<dyn CaptureBackend>, CaptureError>`;
  X11 arm = `X11Backend::connect()?` via the crate's existing From<X11Error> for CaptureError
  (+`inspect_err` warn so the DISPLAY hint — which CaptureError's Display drops — stays in
  default-level logs). Call site treats construct-Err as a rung fall-through (same as
  outputs()-Err). `resolve_cursor()` branches: wayland ladder verbatim in
  `resolve_cursor_wayland()`, X11 = `X11Backend::connect()` + `cursor_pos()` (one-shot
  XQueryPointer), EVERY failure degrades to None (never fails a capture).
- NO new ExecuteError variant: X11Error → CaptureError::Backend{X11} → ExecuteError::Capture
  → exit 4 + backend_tag via the EXISTING mapping (inherited wisdom: "X11 backend errors → 4
  via the existing mapping"); CLI prints the full chain (`{error:#}`) so the hint survives
  one-shot; Probe/Connect variants stay wayland-only.
- `execute/overlay/mod.rs`: **THE DISCOVERY** — `capture --region WxH+X+Y --no-edit` routed
  to the INTERACTIVE overlay child (region preselect rides the request; CLI
  `resolve_selection` keeps it Interactive) → on X11 it failed NoDisplayServer, contradicting
  task 5's mandatory QA, task 8's plan line, the x11 crate's own lib.rs docs, AND task 6's
  SHIPPED message ("headless capture works: ... --region WxH+X+Y --no-edit"). Fix:
  `x11_headless_region()` reroute at the top of `run_interactive` — X11 session + no_edit +
  RegionGeometry-parsable token → `direct::run(Target::Region(region_rect_of(...)))`, the
  launcher one-shot's own window-less mechanism (offset-less WxH centers at resolve_cursor,
  which is now X11-aware; unresolved cursor → Usage error, launcher precedent). ONE seam
  covers daemon-forwarded AND --no-daemon one-shot. Wayland/headless: guard is
  `matches!(detect_session(), Ok(X11))` → always Ok(None) → byte-identical. NOT rerouted
  (honest child failure kept): at-cursor, --last-region/`last`, malformed tokens, any
  interactive form without --no-edit.
- Clipboard → `for_session()`: post/mod.rs (`?` — ExecuteError::Clipboard exists),
  overlay/mod.rs color path (`?`), pin.rs pin_child (Err → `failed(...)` SessionResult),
  settings.rs bridge closure (Err → warn, closure already swallows), ui/examples/pin_window.rs
  (Err → eprintln + exit 1; the sanctioned trivially-safe example switch).
- Telemetry: mod.rs IN_APP_CRATES += flowshot_capture_x11; payload.rs:220 test guard accepts
  DISPLAY (headed); environment.rs `live_machine_reports_the_expected_taxonomy` gained the
  X11 branch (session_type "x11", desktop "other", compositor_version None — live i3 values) —
  it FAILED on this machine pre-task (XDG_SESSION_TYPE=x11 ≠ expected "unknown"), same
  stale-guard class, daemon crate = in scope.

### LIVE QA evidence (i3, DISPLAY=:0, eDP-1 2880x1620 scale 2.25; daemon = debug build,
### RUST_LOG=warn,flowshot_daemon=debug,flowshot_capture_x11=debug; all verbatim)

Daemon X11-leg trace (every capture):
```
DEBUG flowshot_daemon::execute::backend: capture session routing session="x11"
INFO flowshot_capture_x11::probe: X11 session probed; xcb GetImage capture is available randr=1.5 xfixes=Some(ExtensionVersion { major: 5, minor: 0 }) shm=Some(ExtensionVersion { major: 1, minor: 2 })
INFO flowshot_daemon::execute::backend: capture ladder negotiated session="x11" ladder=[X11] desktop=Other excluded=[]
INFO flowshot_daemon::execute::backend: capture backend ready backend=X11 outputs=1
```
1. `capture full --no-edit -o /tmp/x11-task5/` → EXIT=0;
   `2026-10-04_16-54.png: PNG image data, 2880 x 1620, 8-bit/color RGBA, non-interlaced`
2. `capture --region 800x600+100+100 --no-edit -o` → REGION_EXIT=0; reroute log:
   `X11 headless region reroute; no overlay child is spawned target=Region(Rect { x: Logical(100.0), y: Logical(100.0), width: Logical(800.0), height: Logical(600.0) })`;
   PNG = **1800x1350** = 800x600 logical × 2.25 — PHYSICAL-FIRST (#4871: exports are native
   post-transform pixels, never resampled to logical; same as `full` = 2880x1620 physical,
   which the task text itself expects). Task text's "800x600 PNG" was a scale-1 assumption.
   Pixel-exactness PROVEN: fresh `full --raw` f.png + fresh `region --raw` r.png back-to-back,
   `magick f.png -crop 1800x1350+225+225` (= logical 100,100 × 2.25) vs r.png →
   `compare -metric AE` = **0 (0)**.
3. `capture screen eDP-1 --no-edit -o` → SCREEN_EXIT=0; 2026-10-04_17-24.png 2880x1620.
4. `capture full --no-edit --raw > raw.png` → RAW_EXIT=0; `raw.png PNG 2880x1620 8-bit sRGB`.
5. `capture full --no-edit -c` → COPY_EXIT=0; readback via a SECOND independent x11rb
   connection (/tmp/x11-clip-read, x11_smoke wire pattern; xclip/xsel absent per issues.md):
   `clip readback OK: TARGETS=4 image/png bytes=461534 IHDR 2880x1620 saved=/tmp/x11-task5/clip-readback.png`
   — 461 KB > 256 KB INCR chunk cap ⇒ served over INCR reassembly, by the DAEMON process
   (offer outlived the CLI exit; `identify` confirms valid PNG).
6. Interactive sanity: `capture full` (no --no-edit) → EXIT 0 — CORRECT: `full` is the direct
   path on Wayland too (editor only exists for the interactive region form; the QA item's
   "interactive" premise doesn't apply to `full`). Bare `capture` (true interactive) via
   daemon → EXIT=1 with the honest task-6 message: `session child failed (exit 1): no Wayland
   session detected: WAYLAND_DISPLAY is unset or empty. The interactive editor, pins, and
   dialog windows require a Wayland session in this release. On X11 sessions, headless
   capture works: ...`; `grep -c panic daemon.log` = 0.
7. Wayland regression: cannot run live here — the Wayland leg is UNCHANGED code (routing is
   additive in front of it; construct's wayland arms infallible Ok; resolve_cursor_wayland
   verbatim move) + full test suite green (38 suites).

### Findings recorded in issues.md (pre-existing, NOT task-5 causes)
- cli parity_matrix failure (`.omo/evidence/` absent, never committed).
- one-shot `--no-daemon` executor errors are SILENT (exit code only; dispatch.rs/exit.rs
  never print ExecuteError text — surfaced by the X11 interactive failure, applies to any
  Child failure on Wayland too).
- x11 crate lib.rs promises `last --no-edit` headless; reroute covers full|screen|--region
  (incl. offset-less) only — task 9 doc fix or orchestrator decision.
- task 7 clippy sweep list (45 locations).

### Size notes (measured, pure LOC)
- backend.rs 95→**203** (WARNING BAND 200-250: two platform legs + cursor strategies in one
  runner; next edit adding lines should split `backend/x11.rs`). overlay/mod.rs **229**
  (WARNING BAND, was ~190; reroute added ~40). environment.rs **303** (>250 but ≈⅓ is the
  inline #[cfg(test)] mod — negotiate.rs task-1 precedent; lib portion unchanged, my +9 lines
  are test-only). post/mod.rs 229 pre-existing band (1-line change). Others healthy.
- No test added for x11_headless_region: the guard is env-driven (detect_session reads
  process env; no injection seam in daemon code, env-mutation tests race the parallel
  runner); pure rect math is already tested (region_rect_of/launcher); the behavior is
  live-QA-covered above by design.

## [2026-10-04] Task 7: gates green

**Result**: ALL GATES GREEN on this machine. fmt --check exit 0; clippy
--workspace --all-targets -D warnings exit 0 (0 findings); build --workspace
exit 0 with ZERO warnings (members touched to force warning replay);
cargo test --workspace run TWICE consecutively: both exit 0, both
**1438 passed / 0 failed / 0 ignored, 40 suites ok** (no tempdir-flake
recurrence, task 10's fix holds under the new changes); purity-gate PASS
(0 violations, same 2 pre-existing allowlisted hits); cargo deny check
advisories/bans/licenses/sources ALL OK.

### Clippy rust-1.99 sweep: 45/45 fixed (44 line edits + mock.rs:419 block)

Live sweep confirmed EXACTLY the 45 locations task 5 recorded in issues.md
(no drift, no new ones). All are `clippy::assert_is_empty`/`assert_not_empty`
(pedantic, new in rust-1.99) in test code. Fixed mechanically per clippy's own
machine-applicable suggestions: `assert!(x.is_empty())` -> `assert_eq!(x,
[] as [T; 0])`, `assert!(!s.is_empty())` -> `assert_ne!(s, "")` for strings,
`assert_ne!(x, [] as [T; 0])` for the not-empty collection form. No assertion
lost intent; no message args existed to preserve (verified all 45).
- `cargo clippy --fix` ROLLED BACK everything (all-or-nothing per crate):
  clippy's suggested type paths are crate-root-relative (`geometry::OutputCrop`,
  `frame::Frame`, `input::Action`, `command::DaemonCommand`) and don't resolve
  inside test modules (E0433). Applied via a line-verified script instead,
  prefixing `crate::` (universally resolvable; compiler-verified).
- Two suggestions needed hand fixes beyond paths: `render::list::Command` ->
  public re-export `crate::render::Command` (list is a private module), and
  outline.rs `assert_ne!(dots, [])` failed inference (E0282, dots is
  `&[&Command]`) -> `[] as [&Command; 0]`.
- Verified EVERY changed line in the 25 touched src/ files sits after a
  #[cfg(test)]/#[test] marker: ZERO production-code changes.

### Skip mechanism 1: parity_matrix `verified_by_paths_exist_and_meta_points_at_real_files`

`.omo/evidence/` is gitignored (.gitignore:2) local-only QA trail
(docs/verification.md) - never committed, so the test as written could never
pass a fresh clone/CI. Fix: trail-presence-gated enforcement.
- **Deviation from the task's suggested "directory absent -> skip" heuristic,
  forced by reality**: task 8 creates `.omo/evidence/x11-phase-a/` IN
  PARALLEL, so the directory EXISTS on this machine while the historical
  task-16..38 trail it would demand NEVER existed here (issues.md task 5).
  Directory-presence would flip the test to enforcing and keep it red.
- Implemented signal: trail_present := at least ONE matrix-REFERENCED evidence
  file exists on disk. Present (original QA machine) -> every path enforced
  EXACTLY as before (deleted/typo'd evidence still fails; zero weakening).
  Absent (fresh clone / CI / second dev machine / this one) -> evidence-root
  entries skipped with one aggregated `SKIP:` eprintln (count + pointer to
  docs/verification.md). Non-evidence verified_by entries and all meta paths
  stay unconditionally enforced. NOT #[ignore], nothing fabricated, module
  doc updated to state the contract. Live: `SKIP: ... 110 verified_by
  evidence-path checks not run` -> test ok.
- Uses the codebase's existing SKIP convention (daemon_failure_surface.rs,
  tray_bus.rs, portal_shortcuts.rs all `eprintln!("SKIP: ...")` + return).
  eprintln over tracing::warn because tests init no subscriber - a tracing
  note would be silently invisible.

### Skip mechanism 2: ui `cjk_glyphs_resolve_through_fontconfig_fallback`

This machine: `fc-list :lang=zh` = 0 fonts (645 faces total, none CJK) ->
fallback legitimately cannot resolve; environmental. Fix: two-part
discriminator, never faked coverage:
1. Shape first, collect glyph ids. If NO notdef -> full assertions unchanged
   (CJK-capable machines pay zero extra cost, CI ubuntu-latest enforces).
2. If notdef present -> scan the font db (`FontSystem::get_font` ->
   `Font::as_swash().charmap().map('日') != 0`, mirroring cosmic-text's own
   fallback internals; swash not nameable as a dep - method-call resolution
   needs no import, no new dependencies). Any covering face -> assertions run
   at FULL strength (genuinely broken fallback with fonts installed still
   FAILS). No covering face -> `SKIP:` eprintln + early return, documented
   in the test comment. Scan cost measured: 645 faces in 0.50s (debug), once,
   skip path only.
- cosmic-text 0.19 API facts (registry-verified): NO immutable `db()` -
  `db_mut()` only; `get_font(id, weight)` internally `make_shared_face_data`
  (system Source::File faces load fine); AVOID
  `get_font_supported_codepoints_in_word` for coverage - it reads
  `unicode_codepoints()` which is EMPTY unless the `monospace_fallback`
  feature is on (flowshot-ui doesn't enable it) = false negatives.

### Size notes (pure LOC, measured)
- parity_matrix.rs 215 -> **238** (WARNING BAND 200-250; test file, next edit
  adding lines should split the constants/AMENDMENT3 block out).
- text_tests.rs 776 -> **806** (pre-existing >250 inline-test-suite file,
  uniform repo convention - cf. task 1 negotiate.rs precedent; my delta ~30
  lines: the any_font_covers helper + skip gate; NOT refactored mid-plan with
  a sibling task writing evidence in parallel).

### Gate outputs (verbatim tails)
```
fmt exit=0
build exit=0 (0 warnings)
clippy exit=0 (0 findings)
test run1: exit=0 TOTAL passed=1438 failed=0 ignored=0 (40 suites ok)
test run2: exit=0 TOTAL passed=1438 failed=0 ignored=0 (40 suites ok)
purity-gate: 3 crate(s) gated, 0 violation(s), 2 allowlisted hit(s) -> PASS
cargo deny: advisories ok, bans ok, licenses ok, sources ok (exit 0)
```

## [2026-10-04] Task 8: live QA evidence

**Result**: DONE. Full LIVE-verified bundle at `.omo/evidence/x11-phase-a/`
(index.txt + 00-environment + 10 per-check files + 7 PNG artifacts). **All 10
checks PASS, zero production defects.** Headline observed values (all LIVE,
i3/X11 DISPLAY=:0, eDP-1 2880x1620 scale 2.25):

- Env re-measured: RESOURCE_MANAGER root property ABSENT ENTIRELY (no Xft.dpi)
  → mm heuristic 212.65 dpi → 2.25 (probe confirms); SHM 1.2 fd-passing,
  RANDR 1.5, XFIXES 5.0, depth 24 LSBFirst.
- Check 1: `-o` full = 2880x1620; vs `import -window root` oracle:
  4,665,535/4,665,600 px identical; ALL 65 diff px = bar-clock tick
  (2825..2831, 9..17). First naive run invalidated by a USER WORKSPACE SWITCH
  (machine in active use) — third-reader diagnosis (direct example == oracle
  pixel-identical) proved backend correctness; sandwich methodology adopted.
- Check 2: region 800x600+100+100 → 1800x1350 (=logical×2.25); crop of full
  --raw at +225+225 vs region --raw: **diff_pixels=0 / AE=0 (0)** first attempt
  (task-5 result reproduced; crop excludes the bar so the clock can't contaminate).
- Checks 3-5: screen eDP-1 AND screen 0 → 2880x1620 exit 0; --raw = valid PNG
  (file: 8-bit RGBA), vs -o identical except ticking clock (visually confirmed
  19:01:15 vs 19:01:18 — i3status shows SECONDS); --print-geometry →
  `1280x720+0+0` logical (= physical/2.25), stderr empty (one-shot in-process).
- Check 6: `-c` exit 0 → daemon (not the exited CLI) serves TARGETS=4
  [TARGETS,TIMESTAMP,MULTIPLE,image/png], **1,088,822 B over INCR** (4.25× the
  256KiB cap), IHDR 2880x1620, valid PNG saved; `pkill -x flowshot` → readback
  reports NO OWNER; respawn on next call observed. Initial-state bonus: a
  pre-existing daemon was alive ~59 min holding its offer (pinning LIVE-proven).
- Check 7 matrix: 0 (full/screen/region/raw/copy/delay), 1 (bare capture +
  `last --no-edit` — honest task-6 NoDisplayServer message verbatim; last =
  documented Phase A gap despite saved last_region in config), 2 (`--region
  bogus` — in-process usage error, precise message, never reaches daemon;
  legacy `gui`), 4 (DISPLAY=:99 → probe-failure WARN + exit 4). 3/5 unreachable
  on X11 by design; Wayland row hardware-gated here.
- Check 8: shm 38 ms vs forced-plain 80 ms (~2.1×, 18.6 MB/frame) — task-3
  range confirmed. Check 9: -d 1000 exit 0; saved file BYTE-IDENTICAL (768062 B)
  to the clipboard offer served after (actions=[copy,save] both ran).
- Check 10: pinned daemon >> grace (designed); fresh no-hold daemon spawned
  19:17:48 GONE by 19:19:03 (60 s DEFAULT_IDLE_GRACE, lib.rs:118).

**Code change (the one allowed)**: `x11_smoke.rs` gained `read [SAVE_PATH]`
mode — independent CLIPBOARD readback (owner/TARGETS/preferred payload with
INCR reassembly/byte count/PNG IHDR/optional save; no-owner = reported state,
exit 0). Reuses task-4 wire internals via extracted `open_reader()`; smoke mode
regression green. build/clippy `-D warnings`/fmt CLEAN. Pure LOC 186→242
(warning band; example file, single responsibility).

**Methodology learnings for F2 re-runners** (details in issues.md):
- `compare -metric AE` on this IM7 build reports NORMALIZED SUMMED ERROR, not a
  pixel count (calibrated: 10 px @Δ1 → 0.0392157). AE=0 verdicts stay valid;
  non-zero needs the difference-image count method.
- `pkill -f 'flowshot daemon'` self-matches the calling shell → use `pkill -x flowshot`.
- The machine is IN USE: every pixel pair needs a staticity guard (A/B sandwich,
  output suppressed inside the window) + churn attribution (bar clock w/ seconds
  at ~(2825..2838,9..17); user workspace switches; the agent's own scrolling output).
- Config actions=["copy"] means EVERY -o/-c capture pins the daemon; only
  failing calls and one-shot stdout modes (--raw/--print-geometry — they never
  reach the daemon, invoke.rs:108) leave it unpinned. Idle-exit observation
  needs a failing call (e.g. `last --no-edit`) to spawn a no-hold daemon.
- End state: no QA processes, /tmp scratch + example's hardcoded
  /tmp/x11-capture*.png cleaned, root window never modified, daemon absent
  (allowed — auto-spawns).

## [2026-10-04] Task 9: docs

Docs brought in line with the shipped X11 Phase A behavior (evidence:
.omo/evidence/x11-phase-a/index.txt). Gates after the sweep: cargo test
--workspace (40 suites ok, 0 failed, doctests included), cargo fmt --check
clean, ./scripts/purity-gate.sh PASS (0 violations, same 2 pre-existing
allowlisted hits).

Files changed:

- docs/architecture/adr-006-cross-platform-gates.md: status line (X11 Phase 1
  SHIPPED 2026-10-04), platform-crate bullet (two platform crates now),
  actions rationale (clipboard is platform-native: Wayland + X11 backends),
  unsafe allow-list section (SHM fd-passing via memfd + attach_fd >= 1.2,
  plain GetImage fallback, NO SysV shmat, allow-list stays ONE empty entry),
  roadmap item 1 points at the shipped crate, Consequences non-goal amended
  (headless scope shipped; overlay/editor/pins/dialogs Wayland-only pending
  Phase B; no Xwayland fallback; sessions mutually exclusive). Roadmap
  section heading no longer says "no code".
- README.md: intro (X11 headless capture covered, interactive UI
  Wayland-only), cursor bullet ("not from Xwayland"), Xwayland bullet
  ("There is no X11 code path" -> native X11 exists, Wayland never routes
  to it), runtime requirements (Wayland compositor OR X11 server for
  headless), desktop table gains the i3/X11 row after GNOME, probe note
  says "live session", project status notes the Phase A shipment + evidence
  bundle path, non-goal list drops X11. Quick start untouched (no example
  contradicts Phase A scope).
- docs/setup-x11.md: NEW. Requirements (X11 session, RANDR >= 1.2, optional
  XFIXES, optional MIT-SHM >= 1.2), capture backend notes (no negotiation,
  fd-passing fast path, 38 vs 80 ms measured), i3 bindsym snippets
  (headless verbs only; interactive select needs Wayland, stated plainly),
  clipboard (daemon-owned CLIPBOARD, INCR, kill drops the offer by design),
  scale/HiDPI (Xft.dpi preferred else RANDR mm heuristic, 0.25 quantization,
  [1.0, 4.0] clamp, xrdb -merge recipe, documented approximate), known
  Phase A limits (no overlay/editor/pins/dialogs/color picker; last and
  at-cursor keep the honest NoDisplayServer failure; no portal on bare i3;
  XFIXES-composited cursor, software-cursor double-paint caveat).
- docs/porting-roadmap.md: status line (Phase 1 shipped), seam bullet (X11
  no longer a roadmap variant), F20 table cells for the async-wrap and
  request_permission rows note the shipped X11 impl, Phase 1 section marked
  SHIPPED with deviations (x11rb direct, SHM fd-passing >= 1.2 NOT SysV
  shmat/"1.15", no unsafe allow-list growth, scale as specced,
  cursor_events() deferred, last/at-cursor gap recorded); the original
  sketch preserved as a labeled 2026-09-28 record.
- docs/verification.md: QA-machine description gains the i3/X11 session;
  "Landed bundles" entry for .omo/evidence/x11-phase-a/ (LIVE-verified,
  10 checks PASS) under the evidence-trail convention.
- Doc comments only (no behavior): flowshot-capture/src/lib.rs (ladder
  enumeration gains X11, 6 rungs), flowshot-capture/src/backend.rs (X11 no
  longer called roadmap), flowshot-capture-x11/src/lib.rs (drops the
  `last` overpromise; records last/at-cursor as Phase A gaps),
  flowshot-capture-wayland/src/desktop.rs (X11 crate is no longer
  "roadmap"), flowshot-daemon/src/shortcut/detect.rs (Wayland-gate
  rationale reworded), flowshot-ui/src/runtime.rs (both "never falls back
  to X11" spots reworded to Phase A reality), flowshot-ui/src/pins.rs +
  pins/sink.rs + flowshot-actions/src/pin.rs ("Wayland-native" ->
  platform-native), docs/architecture/adr-002-capture-backend-ladder.md
  (status line, probe list, rung 6 added, roadmap-kinds line, non-goal
  bullet), docs/architecture/README.md (five -> six rungs),
  docs/codebase-comparison.md ("Wayland-only v1" -> Wayland + headless
  X11), scripts/purity-gate.sh header comment (task 1's recorded follow-up:
  documents the third invisible form, diagnostic string literals like
  "X11 (xcb GetImage)"; regex NOT widened).

Not touched, deliberately: .omo/plans/x11-support.md (plan checkbox edits
forbidden; its "SHM 1.15" decision text stays as the historical intent -
the correction is recorded in the roadmap's deviation list and here),
.omo/evidence/ (read-only), quick-start examples (accurate as the general
CLI surface; X11 limits live in the table row and setup-x11.md).
