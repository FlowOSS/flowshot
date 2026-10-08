# Issues — x11-support

## [2026-10-04] Task 1 — PRE-EXISTING clippy failure blocks task 7 (NOT caused by task 1)

`cargo clippy -p flowshot-capture --all-targets -- -D warnings` FAILS on
`crates/flowshot-capture/src/mock.rs:419` with `clippy::assert_is_empty`
("used `assert!` to check that a value is empty"; lint added in clippy rust-1.99.0).
`git status` confirms `mock.rs` is UNMODIFIED — this is a toolchain-version regression
in committed code, not a task-1 side effect. Zero clippy findings in `kind.rs` /
`negotiate.rs`.

Out of task 1's scope ("Files modified: kind.rs, negotiate.rs ONLY"), so left alone.
Task 7 (`cargo clippy --workspace --all-targets -- -D warnings`) cannot go green until
it is fixed. Suggested fix, per the lint's own help text — replace
`assert!(backend.capture_outputs(CaptureOpts::default()).await.unwrap().is_empty())`
with `assert_eq!(… .await.unwrap(), [] as [frame::Frame; 0])` so the value prints on
failure. Verify whether the same lint fires elsewhere in the workspace before task 7.

Purity gate: did NOT trip for task 1 — no entry belongs here for it. See learnings.md
for the regex analysis.

## [2026-10-04] Task 4 (resumed) — environment + cross-task findings

1. **`xclip` AND `xsel` are NOT installed on this QA machine** (checked
   `command -v`; also no parcellite/clipit/diodon/python-xlib). Per the task's
   fallback rule, live clipboard verification used a second independent x11rb
   client (`examples/x11_smoke.rs`): real ConvertSelection → SelectionRequest →
   property read-back over the X server, including the 1 MiB INCR path. This
   is wire-identical to what xclip would do, but task 8's live QA checklist
   names `xclip -selection clipboard -t image/png -o` explicitly — either
   install xclip before task 8 or accept the x11rb read-back as the oracle
   there too (and say so in the evidence).
2. **CI runs `cargo deny check` (ci.yml L28) with NO install step** —
   cargo-deny is not preinstalled on ubuntu-latest runners and the workflow
   never installs it, so that CI step cannot pass as written (pre-existing
   gap, unrelated to X11). Locally verified with prebuilt cargo-deny 0.18.9
   (musl binary in ~/.cargo/bin): advisories/bans/licenses/sources ALL OK.
   Fix for task 7: add `EmbarkStudios/cargo-deny-action@v2` (or an install
   step) to ci.yml.
3. **`flowshot-capture-x11` dead_code warnings (task 2's interrupted state)**:
   the crate was member-declared with `src/error.rs` but NO `src/lib.rs`, so
   the ENTIRE workspace failed to load (every cargo command dead). Task 4
   created a minimal scaffold `src/lib.rs` (`#![forbid(unsafe_code)]` +
   `pub mod error;` + SCAFFOLD NOTE) — additive, nothing of task 2's reverted.
   Consequence: `error.rs`'s `connect_error`/`connect_hint` (pub(crate),
   awaiting task 2's connection module) now emit 2 dead_code warnings on
   `cargo check --workspace`. Harmless for builds; **task 7's workspace-wide
   `clippy -D warnings` will trip on them until task 2 lands its connection
   helper** (or task 2 absorbs the functions into real use, which is the plan).
4. **assert_is_empty/assert_not_empty toolchain regression (clippy rust-1.99)
   is workspace-wide, not just flowshot-capture**: task 1 recorded
   capture/mock.rs:419; task 4 found and FIXED 6 more inside flowshot-actions
   (backend.rs:240, pipeline.rs:447+504, encode.rs:96-97, history.rs:151 —
   in-scope because task 4's gate is crate-wide clippy clean).
   **capture/mock.rs:419 remains OPEN** (task 1's suggested fix stands) and
   there may be more in daemon/cli/ui — task 7 should sweep the workspace
   for both lints before running the gate.

## [2026-10-04] Orchestrator findings (batch-1 verification)

- CJK test `editor::tools::text_tests::cjk_glyphs_resolve_through_fontconfig_fallback` FAILS on this
  machine: `fc-list :lang=zh` is EMPTY (no CJK fonts installed). Environmental, pre-existing,
  unrelated to the X11 work. Task 7 must decide: install fonts-noto-cjk on this machine vs
  graceful skip when no CJK font exists. CI (ubuntu-latest) has fonts.
- Task 10 tail: tempdir AtomicU64 imports landed at FILE top in export/path.rs + upload/history.rs
  but the helpers live in #[cfg(test)] modules -> unused-import warnings in lib builds, breaks
  clippy -D warnings. Fix dispatched (move imports into test mod).
- Task 2 tail: fmt violations + 4 clippy doc_markdown errors in flowshot-capture-x11 (interrupted
  agent never ran the gates). Folded into task 3 dispatch.
- Task 6 original quick-agent also pasted spec text into doc comment; fixed via exact-text
  re-dispatch (ses_ef9654b14ffepbTNwrwQaxb414).

## [2026-10-04] Task 3 — capture path: environment deviations + follow-ups

1. **`xsetroot` NOT installed** (also no xwd; ImageMagick `import`/`convert`/`display`
   ARE installed). Per the task's fallback rule the pixel oracle used a solid-color
   WINDOW: `magick -size 640x480 xc:'#FF0000'` shown via `display` on a fresh i3
   workspace, captured live, cross-checked pixel-exact against `import -window root`.
   **The root window was never modified - no restore was needed.** Task 8's QA
   checklist mentions xwd cross-checks: use ImageMagick `import` instead (same
   XGetImage wire path).
2. **Oracle methodology gotcha**: the first oracle run raced the `display` window's
   mapping (capture saw the empty workspace - black center, 4.5M-pixel diff when the
   window appeared mid-run). Task 8: after launching any oracle window, poll
   `i3-msg -t get_tree` for the window rect (or sleep generously) BEFORE capturing.
3. **Software-cursor double-paint (known limitation, not hit here)**: root GetImage
   excludes HARDWARE cursors (verified live: painted glyph differs from import by
   exactly the glyph pixels), but a driver using a SOFTWARE cursor bakes it into the
   framebuffer and paint_cursor would paint it twice. No X11 API excludes it. This
   machine (modesetting) uses a hardware cursor. If task 8 QA sees doubled cursors on
   other hardware, this is why; a heuristic (compare XFIXES serial/position against
   framebuffer content) is Phase B material at best.
4. **Plan decision #3 says "server SHM >= 1.15" - no such MIT-SHM version exists.**
   fd-passing (AttachFd) is **1.2** (Xorg 1.19+; x11rb's own example gates on 1.2;
   this Xorg reports 1.2 live). Implemented gate: >= 1.2. Task 9 must correct the
   plan/ADR text; F1 reviewers: this is a deliberate, evidence-backed deviation from
   the task wording (details in learnings.md).
5. **stitch.rs is a deliberate duplication** of flowshot-capture-wayland/src/stitch.rs
   (sibling platform crates must not depend on each other; flowshot-capture stays
   platform-free and is off-limits per task scope). Follow-up candidate for task 9/F1:
   hoist the platform-free stitch algebra (to_rgba/blit/oriented) into flowshot-capture
   as shared code - out of task 3's allowed file set.

## [2026-10-04] INTERRUPTED STATE — task 5 partial, workspace BROKEN (build exit 101)

Task 5 (daemon wiring) agent was aborted by user (suspected stuck). Tree state at interruption:

LANDED (verified): tasks 1, 2, 3, 4, 6, 10 — plan checkboxes marked. flowshot-capture-x11
complete + live-verified (capture 2880x1620 PNG, SHM 27ms vs plain 88ms, scale 2.25 mm-derived,
cursor via XFIXES). X11 clipboard w/ INCR live-verified. Negotiation ladder has X11 rung.

PARTIAL: task 5 edited ONLY crates/flowshot-daemon/src/execute/backend.rs (+149/-7):
- session routing via flowshot_actions::clipboard::detect_session (SessionKind::X11/Wayland)
- open_x11_session() written (probe_x11 → negotiate → ladder walk, mirrors Wayland leg)
- construct() made fallible with rung fall-through (design improvement over infallible stub)
- MISSING: flowshot-capture-x11 dep in daemon Cargo.toml (THE compile blocker:
  E0433 x3 + cursor_pos on `!`), clipboard call-site switches to for_session() (5 sites),
  telemetry guard (payload.rs:220) + filter (mod.rs:66), live verification.

RESUME PLAN: complete task 5 per .omo/plans/x11-support.md (the partial backend.rs is
reviewable groundwork, not garbage). Then tasks 7 (gates), 8 (live QA), 9 (docs), F1-F4.

ALSO NOTE: .omo/boulder.json is currently deleted from the worktree (uncommitted deletion).
Recreate on resume if tracking desired.

## [2026-10-04] Task 5 (resumed) — findings for tasks 7/8/9 + orchestrator

1. **PRE-EXISTING cli test failure: `parity_matrix::verified_by_paths_exist_and_meta_points_at_real_files`**
   — `tests/parity_matrix.toml` rows carry `verified_by = ".omo/evidence/task-21-flowshot.txt"`
   (and siblings); `.omo/evidence/` does NOT exist on this machine and was NEVER committed
   (no git history). Fails on any fresh checkout. Not task-5-caused (cli untouched). Fix is
   orchestrator's call: commit/restore the evidence dir, or relax the test to skip absent
   evidence roots. Creating placeholder evidence files would be dishonest — not done.
2. **One-shot `--no-daemon` executor errors are SILENT**: `one_shot_capture` →
   `exit::exec_exit_code(&result)` maps ExecuteError → exit code and captures telemetry, but
   NOTHING prints the error text (report() only runs for CliError/anyhow; execute()'s
   tracing::error funnel is bypassed because one-shot calls run_interactive/direct::run
   directly). Surfaced on X11: `capture --no-daemon` interactive → exit 1, empty stderr.
   Pre-existing (applies to every one-shot Child/Capture failure on Wayland too), cli crate,
   outside task 5's change surface. Suggested: print in exec_exit_u8's Err leg or route
   one-shot through execute(). Orchestrator: assign (task 8 visibility or its own fix).
3. **x11 crate lib.rs doc overpromise**: "Phase A headless path: `capture full | screen |
   --region | last` with `--no-edit`" — task 5's reroute covers full/screen (already direct)
   and `--region WxH[+X+Y]` (incl. offset-less cursor-centered); `last --no-edit` and
   `--region at-cursor --no-edit` still fail with the honest NoDisplayServer message on X11.
   Task 9: either correct the crate doc + task-6 message scope, or orchestrator extends the
   reroute (config.capture.last_region → Target::Region is ~8 lines; at-cursor →
   Target::Screen(ScreenTarget::Cursor)).
4. **Task 7 clippy sweep list (rust-1.99 assert_is_empty/assert_not_empty, 45 hits, ALL
   pre-existing)**: core: config.rs:749,775; geometry.rs:1683; scene/arrow.rs:301;
   scene/objects.rs:285. capture: mock.rs:419. daemon: bus.rs:306; instance.rs:259;
   notify/desktop.rs:258,280,297; notify.rs:167; shortcut/portal.rs:241; shortcut.rs:408;
   state.rs:248; telemetry/environment.rs:372-376; telemetry/payload.rs:168;
   tray/outputs.rs:72. ui: chrome/tests.rs:314,329; editor/outline.rs:133;
   editor/registry.rs:88; editor/tests.rs:758,918,923; editor/tools/geometry.rs:353;
   editor/tools/pixelate_tests.rs:255,268,277; pins/tests.rs:149,163,170,220,428,437;
   render/tess.rs:474; selection/tests.rs:534; state/tests.rs:102,113,164,270.
   Task-5 files contributed ZERO (location set identical pre/post).
5. **QA expectation correction for task 8**: on scale≠1 machines `--region 800x600+100+100`
   exports PHYSICAL pixels (800x600 × scale — this machine: 1800x1350, AE=0 vs the full
   capture's crop at physical +225+225). Task 8's checklist should expect logical×scale, not
   literal 800x600 (same principle as `full` = 2880x1620 physical).

## [2026-10-04] Task 8 — live QA: deviations, environment findings, follow-ups

Bundle: `.omo/evidence/x11-phase-a/` (index.txt + 10 check files + 7 PNG artifacts).
ALL 10 CHECKS PASS. No production defects found. Deviations + findings:

1. **`compare -metric AE` is NOT a pixel count on this machine's IM7 build.**
   Calibrated synthetically: two 100x100 images differing in exactly 10 pixels by
   1 level report `0.0392157 (3.92157e-06)` = 10/255 — a normalized summed error,
   not a count. Consequence: **AE=0 remains a valid pixel-exactness verdict**
   (task 5's AE=0 claims stand; check 2 reproduced 0 (0)), but NON-ZERO AE values
   in any past/future evidence from this machine are not pixel counts. Exact
   counts require the difference-image method (`-compose difference` +
   `-threshold 0` + `%[fx:int(mean*w*h+0.5)]`). F2 re-runners: use it.
2. **`xwd` ABSENT** (task text named `xwd -root -silent` as the oracle) —
   ImageMagick `import -window root` used instead (same XGetImage wire path;
   sanctioned by task 3's issues note #1). xclip/xsel still absent (task 4 note
   #1): readback = the new `x11_smoke read` mode (second x11rb client, INCR).
3. **`pkill -f 'flowshot daemon'` (the task-text pattern) kills the calling
   shell** when the pattern appears in the shell's own command line (agent bash
   wrapper self-match — observed: 'Unknown: ChildProcess.kill'). Use
   `pkill -x flowshot`. It did kill the daemon before taking out the shell, so
   the sequence was re-run cleanly on a fresh daemon for verbatim evidence.
4. **Daemon clipboard hold is never cleared by an external SelectionClear.**
   Initial state discovery: daemon PID 9717 (spawned 18:09) still pinned ~59 min
   later by its offer. The x11_smoke self-test superseded the offer (server
   SelectionClear → the daemon's serving thread exits quietly per task-4 design)
   but `DaemonState`'s hold flag stays set — `set_clipboard_offer_held(false)`
   has NO production caller (state.rs L14-17 documents the release observer as
   "whoever observes the release (the CLI/executor wiring)" — unwired). Result:
   the daemon stays pinned for an offer it no longer owns. Direction is
   conservative (never kills a live offer; matches the Wayland limitation where
   wl-clipboard-rs gives no replaced-callback), BUT on X11 the SelectionClear IS
   observable in the serving thread — wiring thread-exit → hold-release is a
   small, sound follow-up (Phase B candidate; NOT a Phase A blocker).
5. **The QA machine is in active use** (user switched workspaces mid-pair —
   invalidated the first naive check-1 run: CLI captured ws1/btop, oracle
   captured ws2/terminal, 9.3% AE; the agent terminal is visible on screen, so
   every command's own output mutates the captured surface). Methodology fix
   (recorded in the bundle): staticity-guarded sandwiches (A/oracle/B back-to-back,
   output suppressed inside the window, A-vs-B is the guard) + retry loops +
   per-pixel attribution. Third-reader cross-check (direct capture example ==
   import oracle, pixel-identical) proved backend correctness during diagnosis.
6. **i3status bar clock shows SECONDS** (visually confirmed: 19:01:15 vs
   19:01:18 in paired frames) — full-frame comparisons ALWAYS straddle a tick on
   this machine; the clock glyph area (~(2825..2838, 9..17)) is a permanent
   ~55-116 px churn source. Region-crop comparisons at y>=225 exclude it
   (check 2's diff_pixels=0). F2: expect the same, don't chase it.
7. **`capture last --no-edit` = exit 1 CONFIRMED LIVE** (task-5 finding #3
   re-verified verbatim in 07-exit-code-matrix.txt): honest NoDisplayServer
   despite a SAVED last_region in config — purely the missing reroute arm.
   `--region bogus --no-edit` = exit **2** (in-process usage error with precise
   expected-format message, never reaches the daemon) — NOT the exit-1 path;
   recorded as the task asked ("record which").
8. **No daemon log file exists** (~/.local/state/flowshot absent) — the
   lifecycle.rs:153 'idle grace elapsed... exiting' line can't be quoted;
   check 10's proof is the pgrep timeline (spawn t+0, gone t+75 s > 60 s grace).

## [2026-10-04] Follow-up: stale daemon after rebuild gives a confusing IO error

A daemon auto-spawned at 20:59 kept the D-Bus name through a 23:46 rebuild (its /proc/exe
-> "(deleted)"). `flowshot capture` then failed with "io failed: No such file or directory"
from the session-child spawn instead of the honest NoDisplayServer message. Kill + rerun
restored correct behavior (exit 1, honest message). ROOT CAUSE: daemon does not detect that
its on-disk binary was replaced (current_exe deleted). Not X11-specific — same class exists
on Wayland. FOLLOW-UP CANDIDATE (Phase B or hygiene): daemon self-check at each dispatch —
if /proc/self/exe is deleted or newer on disk, exit gracefully after the in-flight op and let
the CLI respawn (the auto-spawn path already exists).
