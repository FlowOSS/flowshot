# Plan: X11 support — Phase A (headless capture path)

**Status**: active
**Created**: 2026-10-04
**Owner directive**: "i never stated it's ONLY for wayland" — the ADR-006 "no X11
code path in v1" non-goal is hereby **amended**. X11 support targets the remaining
X11-only desktops/sessions, starting with this machine: i3 on X11, single eDP-1
2880×1620 @ ~212 DPI (HiDPI — the scale-derivation test bed).

## Scope

**Phase A (this plan)**: the headless capture path on X11 sessions —
`flowshot capture full | screen [OUTPUT] | --region WxH[+X+Y]` with
`--no-edit`, all post-capture actions (copy/save/notify/upload/copy-path),
`--raw`, `--print-geometry`, delay, `--instant`-equivalent headless flow.
Clipboard ownership works on X11 (daemon-owned selection, like Wayland).
**Shipped-scope correction (F1 review, 2026-10-04)**: `capture last` and
`--region at-cursor` were listed here but ship as honest exit-1 failures on
X11 (the headless reroute covers only typed `WxH[+X+Y]` tokens); all
user-facing docs state this. Landing them is a Phase B candidate.
Decision-text errata (same review): decision 3's SHM floor is protocol
**1.2** (AttachFd), not "1.15"; decision 6's call-site count is **5**, not 6.

**Explicitly out of scope (Phase B, later plan)**: interactive overlay/region
editor on X11, pins window on X11, settings/consent/launcher dialogs on X11,
`color` picker on X11, cursor-aware preselect via cursor stream (X11 has
XQueryPointer one-shot only in Phase A). Global-shortcut reality on i3 =
`bindsym` snippets (existing `--print-bind-help` fallback path).

## Decisions (locked)

1. **New crate `flowshot-capture-x11`**, sibling of `flowshot-capture-wayland`,
   implementing `flowshot_capture::CaptureBackend`. Core/capture/ui ship with
   only the promotion edits; the purity gate needs zero changes (platform
   crates are not gated).
2. **Ladder position**: `BackendKind::X11` appended LAST in
   `NEGOTIATION_LADDER` (after `PortalScreenshot`). Sessions are mutually
   exclusive — a Wayland probe never reports X11 and vice versa — so position
   is semantic-only and preserves the existing `Ord`-derived invariant
   (`kind.rs` test: all v1 rungs sort before X11).
3. **Capture**: `xcb_get_image` (Z_PIXMAP) as the correctness base via x11rb
   0.14; fast path = MIT-SHM **fd-passing** (`memfd_create` + `shm::attach_fd`
   + `shm::get_image`, server SHM ≥ 1.15), mirroring `icc/shm.rs`. **No SysV
   `shmat`, zero new `unsafe`** — the unsafe allow-list stays at one entry.
4. **Scale derivation** (X11 has no scale concept): prefer `Xft.dpi` from the
   `RESOURCE_MANAGER` root property (scale = dpi/96); fall back to RANDR
   mm-size heuristic (dpi = px / (mm/25.4), scale = dpi/96), rounded to the
   nearest 0.25, clamped to [1.0, 4.0]; documented as approximate. ADR-001
   physical-first rule applies unchanged: capture pixels are truth, logical =
   physical / scale.
5. **Cursor**: one-shot position via `XQueryPointer` (for `screen`/`at-cursor`
   preselect); cursor pixels via `XFixesGetCursorImage`, composited manually
   when `paint_cursor`. `cursor_events()` returns `None` (documented
   degradation, Phase B may add a poll stream).
6. **Clipboard**: `X11Clipboard` implementing the existing
   `flowshot_actions::ClipboardBackend` trait: own `CLIPBOARD` selection from a
   dedicated thread inside the daemon (daemon-ownership model maps 1:1),
   serving `TARGETS`/`TIMESTAMP`/`MULTIPLE` + offer MIMEs, with **INCR**
   transfer for payloads above the request-size limit. `Clipboard::for_session()`
   picks Wayland vs X11 by `WAYLAND_DISPLAY`/`DISPLAY`; the 6
   `Clipboard::wayland()` call sites switch to it.
7. **Session routing in the daemon** (`execute/backend.rs`): if
   `WAYLAND_DISPLAY` set → existing Wayland probe path; else if `DISPLAY` set →
   X11 probe (`CapabilityProbe` with `BackendKind::X11`) + construct
   `X11Backend`; else → existing typed connect error.
8. **UI gate honesty**: `flowshot-ui` `require_display_server()` still gates
   interactive UI to Wayland in Phase A, but `UiError::NoDisplayServer`'s
   message is reworded — it currently claims "never falls back to X11", which
   becomes false the moment Phase A lands. New message names what works on X11
   (headless capture) and what needs Wayland (editor/pins/dialogs).
9. **Permission**: `request_permission() -> PermissionResult::NotRequired`
   (X11 has no capture permission model).
10. **Verification**: LIVE-verified on the i3 session per
    `docs/verification.md`, evidence in `.omo/evidence/`, pixel oracles via
    `xwd`/`import` cross-checks.

## Inherited wisdom (from exploration, 2026-10-04)

- `CaptureBackend` trait: `crates/flowshot-capture/src/backend.rs:99-211`
  (kind/outputs/capture_outputs/capture_region/cursor_events/request_permission).
- `BackendKind`: `crates/flowshot-capture/src/kind.rs` — `is_roadmap()` L64-66,
  `protocol_name()` L84-95, Ord-order test L111-125.
- `negotiate()` + `NEGOTIATION_LADDER`: `crates/flowshot-capture/src/negotiate.rs`
  L46-52, L148-183; 15 data-driven tests L185-390 (several assert roadmap
  rejection of X11 — they must be updated to Windows/MacOs or flipped).
- Reference architecture: `flowshot-capture-wayland` — `worker.rs`
  (spawn_worker bridge, copy pattern), `stitch.rs` (OutputLayout stitch, copy
  pattern), `error.rs` (typed error family pattern), `icc/shm.rs` (memfd +
  file-I/O readback pattern).
- Daemon construction site: `crates/flowshot-daemon/src/execute/backend.rs` —
  `CaptureThread::spawn()` L58, `negotiate()` L62, `construct()` L102-117
  (roadmap match arm at L112), `resolve_cursor_pos` L122.
- Clipboard trait: `crates/flowshot-actions/src/clipboard.rs:64-72`; facade
  `Clipboard` L83-179; `Clipboard::wayland()` call sites:
  `daemon/src/execute/post/mod.rs:82`, `daemon/src/execute/overlay/mod.rs:101`,
  `daemon/src/execute/pin.rs:116`, `daemon/src/execute/settings.rs:137`,
  `flowshot-ui/examples/pin_window.rs:297`.
  `ClipboardError::Transport` wraps `wl_clipboard_rs::copy::Error`
  (`actions/src/error.rs:35`) — must generalize to boxed error.
- UI gate: `flowshot-ui/src/runtime.rs:215-219` (`require_display_server`),
  `flowshot-ui/src/error.rs:17-22` (message).
- Telemetry: `daemon/src/telemetry/payload.rs:220` WAYLAND_DISPLAY guard,
  `daemon/src/telemetry/mod.rs:66` module filter list.
- Gates: `scripts/purity-gate.sh` (gated = core/capture/ui only; comment at L29
  about X11 enum vocabulary may need a touch-up), `deny.toml` (MIT/Apache-2.0
  allowed → x11rb passes), CI = purity + fmt + `clippy -D warnings` + test +
  deny.
- Workspace lints: clippy `all` deny, `pedantic` warn, `unwrap_used` /
  `expect_used` deny; `#![forbid(unsafe_code)]` everywhere except
  flowshot-capture-wayland; edition 2024.

## TODOs

- [x] 1. **Promote `BackendKind::X11` from roadmap to real** in
  `flowshot-capture`: remove X11 from `is_roadmap()`, set real
  `protocol_name()` (`"X11 (xcb GetImage)"`), append to `NEGOTIATION_LADDER`,
  update/extend the negotiate test matrix (X11-only probe → `[X11]`; force X11
  supported/unsupported; empty-probe error now names 6; roadmap tests switch to
  Windows/MacOs). `cargo test -p flowshot-capture` green.
- [x] 2. **`flowshot-capture-x11` crate: skeleton + probe + outputs**:
  workspace member, Cargo.toml (x11rb 0.14 + x11rb-protocol, image, thiserror,
  async-trait, tracing, flowshot-core, flowshot-capture; `[lints] workspace`),
  `#![forbid(unsafe_code)]`, typed `X11Error` family, connection helper,
  `probe_x11()` → `Option<CapabilityProbe>` (DISPLAY connect + RANDR +
  XFIXES + SHM version detection), `outputs()` via RANDR `get_monitors` with
  scale derivation (decision #4) and transform mapping, unit tests for scale
  derivation/parsing (Xft.dpi parse, mm heuristic, rounding/clamp).
- [x] 3. **`flowshot-capture-x11` capture path + CaptureBackend impl**:
  `capture_outputs` (GetImage Z_PIXMAP; SHM-fd fast path when server ≥1.15,
  plain fallback otherwise; per-output honor of RANDR transform; pixel-format
  mapping verified against a live solid-color oracle), `capture_region` via
  OutputLayout stitch (mock.rs/wayland stitch.rs pattern), one-shot
  `cursor_pos()` (XQueryPointer) + XFixes cursor image compositing under
  `paint_cursor`, `request_permission` = NotRequired, `cursor_events` = None,
  `kind` = X11. `examples/probe.rs` + `examples/capture.rs` for live
  diagnostics.
- [x] 4. **X11 clipboard** in `flowshot-actions`: `clipboard/x11.rs`
  (`X11Clipboard` impl `ClipboardBackend`: selection ownership + dedicated
  serving thread + TARGETS/TIMESTAMP/MULTIPLE + INCR for large payloads),
  generalize `ClipboardError::Transport` to boxed source, `Clipboard::x11()` +
  `Clipboard::for_session()` (WAYLAND_DISPLAY → wayland, else DISPLAY → x11,
  else typed error). Unit tests where headless-possible (offer→targets mapping,
  INCR chunking math); live serving verified in task 8.
- [x] 5. **daemon/cli composition wiring**: session-type branch in
  `execute/backend.rs` (Wayland probe vs X11 probe per decision #7),
  `construct()` arm for X11, X11 one-shot cursor-pos wiring into the preselect
  path (parallel to `resolve_cursor_pos`), clipboard call sites →
  `Clipboard::for_session()` (5 daemon sites; ui example stays `wayland()` but
  gains a comment, or switches if trivially safe), telemetry guard
  (`payload.rs:220` accepts DISPLAY) + filter list adds
  `flowshot_capture_x11`. `flowshot-cli` + `flowshot-daemon` Cargo.tomls gain
  the `flowshot-capture-x11` dep.
- [x] 6. **UI gate honesty** (`flowshot-ui`): reword
  `UiError::NoDisplayServer` (no more "never falls back to X11"; state what
  works headless on X11, what needs Wayland). `require_display_server` logic
  UNCHANGED (interactive UI stays Wayland-gated in Phase A). Update affected
  tests/snapshots.
- [x] 7. **Gates green**: `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `./scripts/purity-gate.sh`, `cargo deny check`
  (x11rb MIT/Apache-2.0). Fix whatever trips; Cargo.lock committed.
- [x] 8. **Live i3 QA** per `docs/verification.md` (LIVE-verified bundle in
  `.omo/evidence/x11-phase-a/`): on i3/eDP-1 2880×1620 —
  `capture full --no-edit -o` pixel-cross-checked vs `xwd`/`import`;
  `capture screen eDP-1 --no-edit -o`; `capture --region 800x600+100+100
  --no-edit -o` geometry-correct; `--raw` stdout PNG decodes;
  `--print-geometry`; `-c` copy then paste verified
  (`xclip -selection clipboard -t image/png -o`); clipboard survives GUI-exit
  (daemon-owned); scale derivation value recorded (Xft.dpi present? mm
  heuristic result); exit codes 0/3/4 sanity; daemon idle-exit still works.
- [x] 9. **Docs**: amend ADR-006 (X11 non-goal lifted, Phase 1 shipped, SHM
  fd-passing decision recorded, unsafe allow-list unchanged); README desktop
  table gains an X11/i3 row + feature footnotes (Phase A headless scope);
  new `docs/setup-x11.md` (i3 binds, clipboard notes, scale caveats);
  `docs/porting-roadmap.md` Phase 1 marked shipped with deviations;
  `docs/verification.md` X11 evidence entry.
- [x] 10. **Discovered: test-hygiene flake** — `unique_tempdir()` (pid +
  nanos) collided under full-workspace load:
  `copy_path_after_copy_appends_uri_list_to_image_offer` failed at
  pipeline.rs:433 (`saved.exists()`) when a sibling test's `remove_dir_all`
  hit the same dir. Harden all three copies
  (`clipboard/pipeline.rs:327`, `export/path.rs:106`, `upload/history.rs:124`)
  with an atomic counter (or tempfile crate if already in the tree);
  prove with a stressed repeat run (`cargo test -p flowshot-actions` ×20).

## Final Verification Wave

- [x] F1. **Independent code review** of the full X11 diff (trait conformance
  vs porting-roadmap F20 checklist, error typing, no shortcuts): APPROVE/REJECT
- [x] F2. **Independent live QA re-run** on the i3 session reproducing the task-8
  evidence from scratch: APPROVE/REJECT
- [x] F3. **Gate parity audit** (purity-gate, clippy, fmt, full tests, deny;
  CI symmetry; confirm Wayland paths regression-free): APPROVE/REJECT
- [x] F4. **Docs accuracy review** (README/ADR/roadmap claims match shipped
  code + evidence; no overclaiming): APPROVE/REJECT

## Success criteria (commands)

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
./scripts/purity-gate.sh
cargo deny check
# Live on i3:
target/debug/flowshot capture full --no-edit -o /tmp/x11-qa
target/debug/flowshot capture --region 800x600+100+100 --no-edit -c
```
