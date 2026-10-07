# Plan: X11 Phase B — interactive UI on X11

**Status**: implemented, independently reviewed, review fixes landed
(2026-10-07, draft MR #2); F2 live re-run remains OPEN on the QA machines
**Created**: 2026-10-04
**Owner directive**: "do phase B or whatever, duh" — the interactive experience
(overlay region select, editor, pins, dialogs, color picker) on X11 sessions.
Target machine: i3 on X11, eDP-1 2880x1620 @ scale 2.25, picom present
(PID 13180; `_NET_WM_CM_S0` query inconclusive), xdotool 4.x available for
synthetic-input QA. Phase A (headless capture) is shipped and verified
(see .omo/plans/x11-support.md, evidence in .omo/evidence/x11-phase-a/).

## Scope

**Phase B (this plan)**: `flowshot capture` (interactive region select +
editor), `flowshot pin`, `flowshot color`, `flowshot settings`, the launcher
dialog, and the consent dialog all work on X11 — verified live on i3.
Plus the recorded riders: `capture last` / `--region at-cursor` headless
reroutes, clipboard hold-release on SelectionClear, stale-daemon self-check.

**Out of scope**: Windows/macOS ports, X11 cursor event stream (the overlay
reads its own pointer events; not needed), multi-monitor X11 overlay live QA
(single-panel machine — hardware-gated; the code path mirrors Wayland's
per-output windows).

## Decisions (locked)

1. **Winit-only overlay first** (librarian-verified capability matrix):
   `Fullscreen::Borderless` + `WindowLevel::AlwaysOnTop` work on X11 via EWMH
   (`_NET_WM_STATE_FULLSCREEN` / `_NET_WM_STATE_ABOVE`), and winit auto-calls
   `set_input_focus` when entering fullscreen — keyboard focus and Esc work
   without grabs. **Contingency (documented, not planned)**: if live focus
   proves unreliable on i3, fall to override-redirect windows
   (`WindowAttributesExtX11::with_override_redirect`) + x11rb grabs.
2. **Transparency is not load-bearing**: the overlay backdrop is the frozen
   capture (opaque, covers the window). Keep `with_transparent(true)` at
   first live bring-up; if a compositor-less session renders black, the X11
   customizer arm sets `with_transparent(false)` — decide from live evidence,
   record the decision.
3. **One session gate fix**: `require_display_server()` (flowshot-ui
   runtime.rs:218) accepts `DISPLAY` as well as `WAYLAND_DISPLAY`;
   `UiError::NoDisplayServer` fires only when NEITHER is set, message reworded
   accordingly. All six interactive paths (overlay, pin, settings, launcher,
   consent, color) inherit the fix.
4. **One shared customizer helper in the daemon**: e.g.
   `session_window_customizer(app_id, title)` branching on
   `flowshot_actions::clipboard::detect_session()` — Wayland arm keeps
   `WindowAttributesExtWayland::with_name`, X11 arm uses
   `WindowAttributesExtX11::with_name` (WM_CLASS). All 5 production sites
   (overlay session.rs:71, pin.rs:135, launcher/mod.rs:146, settings.rs:85,
   consent.rs:181) + 3 ui examples switch to it. flowshot-ui itself gains NO
   platform imports (purity gate holds).
5. **Launcher output probe**: `execute/launcher/mod.rs:175-186` probes via the
   Wayland CaptureThread — add the X11 leg via `X11Backend::outputs()`.
6. **Riders folded into the right files**: `capture last` / `--region
   at-cursor` headless reroutes extend `x11_headless_region` (overlay/mod.rs —
   same seam, task 2 owns that file). Clipboard hold-release on SelectionClear
   (wire the X11 serving thread's exit → `set_clipboard_offer_held(false)`;
   F1 concern #2) and stale-daemon self-check (detect /proc/self/exe deleted
   or older than the resolved binary → graceful exit after in-flight op; the
   auto-spawn path respawns) are daemon-internals, task 4.
7. **Live QA drives the overlay with xdotool** synthetic input (present on
   this machine) and captures evidence via FlowShot's own headless capture of
   the visible overlay. This machine is in active use — QA runs must be
   timeout-bounded and self-reversing per docs/verification.md.

## Inherited wisdom (Phase B exploration, 2026-10-04)

- Overlay windows: `crates/flowshot-ui/src/handler/mod.rs:165-172`
  (`overlay_window_attributes`: fullscreen borderless, transparent, no
  decorations, AlwaysOnTop), spawned one-per-monitor at handler/mod.rs:27-85.
- Pin windows: `crates/flowshot-ui/src/pins/spawn.rs:128-139` (fixed size,
  transparent, AlwaysOnTop). Dialogs are normal decorated windows.
- `require_display_server` call sites: runtime.rs:96+136, pins/runtime.rs:154,
  settings/window/runtime.rs:88, launcher/window.rs:163, consent/window.rs:74.
- WindowCustomizer: `flowshot-ui/src/pins/runtime.rs:38-61`; applied at
  handler/mod.rs:46-50 (overlay), pins/spawn.rs:52-56, settings/window/
  app.rs:61-64, consent/app.rs:72-75, launcher/app.rs:70-73.
- Overlay session flow: daemon execute/overlay/session.rs `run_overlay_session`
  (customizer at L71-74); frozen frames via `flowshot_ui::capture_frozen`;
  per-output monitor matching by connector name then origin (ui monitor.rs:40-70).
- Session child: execute/session/mod.rs — spec JSON, spawn, main-thread winit
  loop (X11 main-thread requirement already satisfied by this model).
- winit X11 facts: `WindowAttributesExtX11` = with_name/with_override_redirect/
  with_x11_window_type/with_base_size/...; fullscreen auto-focuses
  (x11/window.rs:678-685); `focus_window()` = `_NET_ACTIVE_WINDOW`;
  `set_cursor_grab(Confined)` = XGrabPointer (Locked unsupported); no keyboard
  grab API (unneeded under fullscreen auto-focus); IME via XIM works.
- Watch items: winit X11 monitor names come from RANDR (should match our
  connector names; origin fallback exists); `drag_window()` on X11 uses
  `_NET_WM_MOVE_RESIZE` (i3 honors it); frame pacing `pre_present_notify` is
  a no-op on X11 (fine).

## TODOs

- [x] 1. **Session gate + session-aware customizer helper**: ui
  `require_display_server` accepts DISPLAY (decision #3) + message reword +
  tests; daemon `session_window_customizer(app_id, title)` helper (decision #4)
  + all 5 production sites + 3 ui examples switched; launcher probe_outputs
  X11 leg (decision #5). Proof: `flowshot settings` and the consent dialog
  OPEN on i3 (normal windows — the first visible X11 UI), screenshotted via
  FlowShot's own headless capture.
- [x] 2. **Interactive overlay + editor on X11**: live bring-up of
  `flowshot capture` (overlay windows per output, keyboard focus, Esc cancel,
  region select, editor tools, save/copy completion), the transparency decision
  (decision #2), color picker (`flowshot color`), AND the `capture last` /
  `--region at-cursor` reroutes (decision #6 — same file/seam). xdotool-driven
  QA evidence per check.
- [x] 3. **Pins on X11**: `flowshot pin` window lifecycle live on i3 (spawn,
  drag, zoom-to-cursor, opacity via digit keys, close), customizer via the
  task-1 helper; record any `_NET_WM_MOVE_RESIZE` quirks.
- [x] 4. **Robustness riders**: clipboard hold-release on SelectionClear
  (X11 serving thread exit → daemon `set_clipboard_offer_held(false)`; Wayland
  parity kept — if Wayland has no equivalent signal, document the asymmetry);
  stale-daemon self-check (exe deleted/superseded → finish in-flight, exit,
  let respawn happen; issues.md 2026-10-04 entry is the spec).
- [x] 5. **Gates green**: fmt, `clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace` ×2, purity-gate, deny. — 2026-10-07 at tip
  `faf23a0`: `just check` exit 0 (session env); second full workspace test
  run headless (only failure: the pre-existing dev-box-only
  `live_machine_reports_the_expected_taxonomy` env artifact —
  `XDG_SESSION_TYPE` survives the scrub; CI runners lack it); the GitHub
  Actions run at tip is fully green (purity, fmt, clippy, tests, deny).
- [x] 6. **Live QA evidence bundle** at `.omo/evidence/x11-phase-b/` per
  docs/verification.md: the full interactive matrix (overlay select→save/copy,
  editor tools smoke, Esc cancel, pins, color, settings, launcher, consent if
  triggerable, last/at-cursor reroutes, riders' proofs), xdotool-driven,
  timeout-bounded, self-reversing.
- [x] 7. **Docs**: README X11 row loses "headless only" (interactive ships);
  setup-x11.md gains the interactive section (incl. picom/transparency note +
  focus behavior on i3); ADR-006 + porting-roadmap Phase-B status;
  verification.md entry; the Phase A "Phase B candidates" notes updated.
  — 2026-10-07: landed in `ac27911` (verification.md Phase B entry),
  `6e54e3b` (all twelve stale claims + the interactive section), `a282946`
  (citation convention), `15e0120` (two-machine naming).

## Final Verification Wave

- [x] F1. **Independent code review** of the Phase B diff (purity gate holds —
  zero platform imports in gated crates; session routing discipline; error
  typing; no shortcuts): **REQUEST_CHANGES → addressed.** Four scoped
  reviewers 2026-10-07 (new crate / daemon / cross-crate / shipping
  hygiene; reports in `.omo/evidence/*code-review.md`, ledger in draft
  MR #2): 9 merge blockers + minors found; every one fixed in a named
  commit on the branch or explicitly deferred with reasons. A fresh
  re-review pass remains advisable before un-drafting.
- [ ] F2. **Independent live QA re-run** on the i3 session reproducing the
  task-6 evidence from scratch: APPROVE/REJECT — **OPEN: needs the i3/X11
  laptop (and should cover the Wayland pin-zoom live check, branch
  notepad item 10); cannot run from the workstation.**
- [x] F3. **Gate parity audit** (CI symmetry, Wayland regression-free by
  construction, lockfile hygiene): **RESTORED/FIXED.** CI symmetry was
  broken repo-wide (pre-existing walls: dead rfd→glib-sys `1301d33`,
  libspa-sys headers `176c602`, never-installed cargo-deny + clippy
  parity `136324a`; tip run fully green). The one real Wayland regression
  the audit class exists for (pin zoom) was found, fixed and test-guarded
  (`1f114a5`, `c3401a3`); lockfile hygiene: phantom/misplaced deps out
  (`4341ddd`).
- [x] F4. **Docs accuracy review** (claims match code + evidence):
  **REJECT → fixed.** The hygiene review found 12 false Phase-A claims,
  a committed temp artifact, six dangling evidence citations and the
  two-machine conflation; all resolved (`6e54e3b`, `69061fa`, `a282946`,
  `ac27911`, `15e0120`, `e312a4d`, `5d2f992`).

## Success criteria (commands)

```sh
cargo build --workspace && cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check && ./scripts/purity-gate.sh && cargo deny check
# Live on i3 (the money tests):
target/debug/flowshot capture            # overlay opens; select; editor; save
target/debug/flowshot pin                # pin floats, drags, closes
target/debug/flowshot color              # pick a pixel, hex to clipboard
target/debug/flowshot settings           # settings window opens
```
