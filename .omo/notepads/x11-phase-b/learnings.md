# Learnings — x11-phase-b

## 2026-10-04 Exploration synthesis (2 explore + 1 librarian agents)

- flowshot-ui is already platform-free (purity gate): overlay windows are
  Fullscreen::Borderless + transparent + undecorated + AlwaysOnTop, one per
  monitor (handler/mod.rs:165-172); input/IME/wgpu all portable. The ONLY
  ui-crate change: require_display_server (runtime.rs:218-223) accepting DISPLAY.
- WindowCustomizer seam (pins/runtime.rs:38-61) is the injection point; 5
  production sites use WindowAttributesExtWayland::with_name — need an X11 arm
  (WindowAttributesExtX11::with_name = WM_CLASS). One shared helper in daemon.
- Launcher probe_outputs (execute/launcher/mod.rs:175-186) uses the Wayland
  CaptureThread — needs an X11Backend::outputs() leg.
- winit 0.30 X11: Fullscreen::Borderless → _NET_WM_STATE_FULLSCREEN +
  AUTO set_input_focus (keyboard/Esc work without grabs); AlwaysOnTop →
  _NET_WM_STATE_ABOVE; override-redirect available (contingency only);
  CursorGrabMode::Confined = XGrabPointer, Locked unsupported; no keyboard
  grab API (unneeded under fullscreen auto-focus); IME via XIM works;
  transparency needs 32-bit visual + compositor.
- This machine: picom RUNNING (PID 13180) but `xprop -root _NET_WM_CM_S0`
  said "not found" — compositor state uncertain; transparency decision is
  made from live bring-up evidence (plan decision #2). xdotool 4.x present.
- Overlay transparency is not load-bearing: backdrop = frozen capture (opaque).

## [2026-10-04] Task 1: gate + customizer + launcher probe

### A. Session gate (flowshot-ui)
- `require_display_server` (runtime.rs) now delegates to an injectable
  `has_display_server(wayland_display, display)` pure fn (mirrors the
  flowshot-actions `session_from_env` precedent): either var set-nonempty
  passes; empty=unset; NEITHER -> `UiError::NoDisplayServer`. 6 unit tests
  lock the rule (Wayland-only, X11-only, XWayland-both, empty-Wayland+DISPLAY,
  neither, both-empty) + 1 message-token test (repo precedent:
  `no_qualified_adapter_message...`). Purity holds: env-var reads only,
  purity-gate PASS (0 violations).
- Message reworded: "no display server detected: neither WAYLAND_DISPLAY nor
  DISPLAY is set (or both are empty); the interactive UI needs a Wayland or
  X11 session". Variant KEPT (callers match on it). Doc sweep: runtime.rs
  (new/with_window_customizer/gate), consent/window.rs, launcher/window.rs,
  settings/window/runtime.rs, pins/runtime.rs (WindowCustomizer seam doc),
  lib.rs (purity-contract paragraph) - all now say WAYLAND_DISPLAY/DISPLAY.
- FINDING (tasks 2/3/4, check your fixtures): TWO integration tests built
  "headless" envs by scrubbing ONLY `WAYLAND_DISPLAY` -
  flowshot-cli/tests/session_dispatch.rs and
  flowshot-daemon/tests/consent_session.rs. On this X11 machine the ambient
  `DISPLAY=:0` leaked in, the child passed the NEW gate, opened a real window
  on the live desktop and hung the 60s budget (session_dispatch FAILED live;
  consent_session would have popped the dialog mid-suite). Fixed both with
  `.env_remove("DISPLAY")` - intent preserved (headless -> typed fast fail,
  no nested-runtime panic), fixtures now actually headless. Any future
  "headless" fixture must scrub BOTH vars.

### B. session_window_customizer (flowshot-daemon)
- New `execute/window.rs`: `pub(crate) fn session_window_customizer(app_id,
  title) -> WindowCustomizer`, exhaustive match on `detect_session()`:
  `Ok(X11)` -> `WindowAttributesExtX11::with_name`, `Ok(Wayland)|Err(_)` ->
  the unchanged `WindowAttributesExtWayland::with_name` (gate errors first on
  no-session; backend.rs routing precedent). Both winit ext traits define
  `with_name` on WindowAttributes -> trait imports are BRANCH-SCOPED (a
  shared scope is E0034 ambiguity).
- winit 0.30 API CONFIRMED from vendored source
  (~/.cargo/registry/src/*/winit-0.30.*/src/platform/x11.rs:159):
  `fn with_name(self, general: impl Into<String>, instance: impl Into<String>)`
  - identical signature to the Wayland ext; WM_CLASS = "instance", "general".
- All 5 production sites switched (overlay/session.rs, pin.rs,
  launcher/mod.rs, settings.rs, consent.rs); zero direct `with_name`/
  `ExtWayland` left in daemon src outside window.rs (grep-verified).
- 3 ui examples (consent_dialog, launcher_dialog, pin_window): flowshot-daemon
  is UNREACHABLE from flowshot-ui examples (daemon deps on ui -> dev-dep would
  cycle; and the helper is pub(crate) by design) -> duplicated the branch as a
  local `session_window_customizer` fn per example with a pointer comment at
  the helper (task-sanctioned fallback; recorded here). Examples are outside
  the purity gate's scan (src/ only) - verified gate still PASS.

### C. Launcher probe X11 leg
- `probe_outputs` (launcher/mod.rs) routes on `detect_session()`: X11 ->
  `probe_outputs_x11` = `X11Backend::connect()?` + `outputs().await` driven by
  `futures::executor::block_on` (the x11 worker futures are runtime-agnostic -
  futures::channel oneshot + std thread; the session child has NO ambient
  executor, query_system_theme precedent). Wayland/no-session -> the unchanged
  CaptureThread leg. Both legs keep warn-and-empty degradation.
  `From<X11Error> for CaptureError` exists (error.rs:87) so `?` unifies.

### LIVE i3 proof (the money test) - PASSED
- Env: DISPLAY=:0, no WAYLAND_DISPLAY, XDG_SESSION_TYPE=x11, picom 13180.
  No stale daemon present; auto-spawn used the fresh binary (built 01:29).
- `target/debug/flowshot settings`: window appeared in ~1s (WID 39845890).
  xprop: `WM_CLASS(STRING) = "FlowShot Settings", "flowshot-settings"` (the
  X11 customizer arm proven live), `WM_NAME = "FlowShot Settings"`,
  `_NET_WM_STATE = MAXIMIZED_VERT, FOCUSED`, _MOTIF_WM_HINTS decorations ON.
  `xdotool getwindowfocus` = same WID -> i3 AUTO-FOCUSED it, no focus quirk,
  no override-redirect needed for dialogs. i3 tiled it (MAXIMIZED_VERT is
  i3's tiling state, not a winit ask).
- While open: `target/debug/flowshot capture full --no-edit -o /tmp/b1/`
  exit 0 -> EVIDENCE PNG: /tmp/b1/2026-10-05_01-30.png (801KB, full panel).
  Image extraction verified the settings window VISIBLE in-frame (General/
  Interface/Filename Editor/Shortcuts tabs, indigo accent, i3 status bar
  showing "FlowShot Settings"). Headless capture does NOT contend with the
  settings window-session gate (no window spawned -> no gate acquire).
- BONUS: the daemon-startup consent dialog also opened on i3 in the same run
  (WID 37748738, `WM_CLASS = "Help improve FlowShot?", "flowshot-consent"`) -
  plan todo 1's second window proven. Dismissed via WM_DELETE (xdotool
  windowkill): no answer recorded -> prompt re-arms (clean dismissal).
- Self-reversal: both windows closed, zero flowshot windows remain, the
  auto-spawned daemon was killed after QA (pre-QA state restored; matters
  because task 4 rebuilds in parallel and the stale-daemon self-check does
  not exist yet).
- Wayland unchanged (code inspection): Wayland arm keeps
  `WindowAttributesExtWayland::with_name(app_id, title)` with identical args;
  `WAYLAND_DISPLAY`-wins routing means XWayland sessions take the same arm as
  before. Cannot live-test Wayland on this machine.

### Gates
- cargo build/test/clippy --workspace --all-targets -D warnings / fmt --check
  / purity-gate.sh: ALL GREEN (full workspace suite 0 failed; new gate tests
  observed running). LOC: window.rs 17 pure, runtime.rs 147, launcher/mod.rs
  160 - all under budget.

### Notes for later tasks
- Task 7 (docs): flowshot-capture-x11/src/lib.rs:11-13 crate doc still claims
  interactive paths "fail with the honest NoDisplayServer message" - stale
  after this gate change (left untouched; docs are task 7's sweep). README
  "Wayland-only in this release" rows also await task 7.
- Tasks 2/3: consume `crate::execute::window::session_window_customizer`
  (overlay session.rs already routes through it - task 2 must not re-hardcode
  the Wayland ext when touching overlay spawn code).

## [2026-10-04] Task 4: riders (executed overnight into 2026-10-05 02:0x)

### A. Clipboard hold-release on SelectionClear — WIRED, live-verified

Shape chosen (minimal honest fit for the existing trait):
- `OfferLossHook = Arc<dyn Fn() + Send + Sync>` in flowshot-actions clipboard.rs;
  `X11Clipboard::with_loss_hook(hook)` + `Clipboard::for_session_with_loss_hook(hook)`.
  The `ClipboardBackend::serve` trait is UNCHANGED — the Wayland impl is untouched.
- Process-wide `OWNER_EPOCH: AtomicU64` in x11.rs: every serve() claims an epoch
  BEFORE its owner takes the selection; the serving thread fires the hook on exit
  (SelectionClear / connection death) ONLY if its claim is still the latest. This
  makes self-supersede silent — critical because the daemon builds a FRESH
  Clipboard per capture (post/mod.rs), so the previous capture's thread always
  exits via SelectionClear; without the epoch every second capture would falsely
  clear the new hold.
- Daemon side: `execute/post/sinks.rs` gained the hold-release bridge
  (`clipboard_for_run` → clipboard + `lost: Arc<AtomicBool>` latch; hook body
  `hold_release_hook` latches + `set_clipboard_offer_held(false)`). post/mod.rs
  sets held(true) only when `Copied && !lost` — the latch closes the ordering
  hazard of a takeover DURING the pipeline run (hook fires before the report).
  Residual ns-scale race (hook lands between lost.load() and held(true)) degrades
  to the old never-clear behavior; documented in code, accepted.
- Wayland asymmetry (documented in OfferLossHook docs + state.rs module docs +
  sinks.rs bridge docs): wl-clipboard-rs has no replaced callback → hook never
  fires → Wayland keeps the conservative never-clear hold. No Wayland behavior change.

LIVE timelines (i3/X11, this machine):
1) Log-evidence run (manual `daemon --auto-spawned --idle-grace 15`, RUST_LOG debug):
   - 02:04:23.831 daemon PID 3850 started; 02:04:26.701 capture exit 0, x11 ownership acquired (window 0x2600000)
   - 02:04:46.713 T+20s (grace 15s EXCEEDED): daemon ALIVE → pinned by the hold
   - readback pre-takeover: `clip readback OK: owner=0x2600000 TARGETS=4 [...image/png] bytes=767275 IHDR 2880x1620`
   - 02:04:49.399 TAKEOVER: x11_smoke serves a new offer (independent x11rb client → server SelectionClear)
   - daemon log chain (296 µs total): 02:04:49.625871 "x11 clipboard ownership lost; serving thread exiting"
     → .625989 "clipboard offer lost to another client; releasing the clipboard-offer hold"
     → .626118 lifecycle "idle grace elapsed with no persistence reason - exiting" (idle_secs=25)
     → .626167 "daemon shutting down reason=IdleExit"
   - 02:04:49.648 pgrep: daemon GONE (<0.5 s after takeover)
2) Production shape (auto-spawned helper, default 60 s grace, pgrep evidence):
   - 02:06:52.134 T0 `capture full --no-edit -c` → helper PID 5804 (capture exit 0 @ 02:06:54.884)
   - 02:08:05.932 T0+~74 s (PAST the 60 s grace): PID 5804 ALIVE → pinned by clipboard-offer hold
   - 02:08:05.954 T1 takeover via x11_smoke (exit 0 @ 02:08:06.265)
   - 02:08:06.291 T1+0.34 s: daemon GONE → hold released, idle-exit fired
   Takeover method = the task's suggested `cargo run -p flowshot-actions --example
   x11_smoke` (smoke mode serves its own text offer; no xclip/xsel on this machine).
   x11_smoke regression: PASS pre-test (02:03:28 "direct text, TARGETS (8), supersede, 1024 KiB INCR").

### B. Stale-daemon self-check — IMPLEMENTED, live-verified

Shape chosen: dispatch-time gate in bus.rs `accept()` → park-and-exit (NO reply):
- New `src/stale.rs`: `ExeIdentity {device, inode, modified}` baseline snapshot at
  startup (stat /proc/self/exe); per-dispatch probe = readlink (current_exe) +
  stat; pure decision `exe_is_stale`: stale iff link ends " (deleted)" OR identity
  moved off baseline (inode swap / in-place mtime change). Failing probe = NEVER
  stale (conservative). Linux-/proc-honest: no /proc → inert gate. Non-unix cfg
  stub keeps the crate's cross-platform shape.
- `SupersessionGate {baseline, exit: Arc<Notify>}`; DaemonOptions gained
  `supersession_baseline` QA knob (bus_address precedent) → tests inject a foreign
  baseline. New `ShutdownReason::Superseded`; run() select gained the branch.
- accept(): stale → warn log + trigger_exit + `std::future::pending()` (DELIBERATELY
  never replies). The shutdown releases the name; the BROKER answers the pending
  call; the CLI's single dispatch retry re-handshakes → name free → spawns a fresh
  daemon → transparent success. Replying before shutdown would race the name
  release (retry could reach the dying daemon); the broker's answer is causally
  ordered AFTER the release → race-free by construction.
- **EMPIRICAL FINDING (pins broker behavior)**: dbus-daemon answers a call that
  was PENDING when the owner's connection disconnected with
  `org.freedesktop.DBus.Error.NoReply` — NOT `NameHasNoOwner` (that one is for
  calls made when no owner exists). Verified by tests/supersession.rs (private
  dbus-daemon broker; arrived in 0.06 s — also proves close() does NOT deadlock
  on the parked handler). Consequences: CLI dispatch.rs retry predicate extended
  to `OWNER_VANISHED_ERRORS = [NameHasNoOwner, NoReply]` (safe: the daemon's 5 s
  startup-reply window ≪ the 25 s bus timeout, so NoReply from a FlowShot daemon
  always means "died mid-call", never "slow"); instance.rs classify_method_error
  now maps NoReply → OwnerVanished (second-instance path honesty).
- Tests: 9 headless unit tests (pure decision table incl. deleted-suffix-with-
  matching-identity — the cargo-rename case the identity compare CANNOT see;
  live /proc gate; trigger wakes watcher) + tests/supersession.rs broker
  integration (exits Superseded, command NEVER served, client gets NoReply,
  no hang).

LIVE timeline (i3/X11):
- 02:09:34.885 T2 `capture full --no-edit -c` → daemon PID_A=7503 (exit 0 @ 02:09:37.657; exe clean)
- 02:09:51.528 T3 `touch crates/flowshot-cli/src/main.rs && cargo build` (13.8 s, binary replaced)
- evidence: PID 7503 alive; `readlink /proc/7503/exe` → `/home/.../target/debug/flowshot (deleted)`;
  mechanism proof: spawning that path → "No such file or directory" (the exact issues.md symptom class)
- 02:10:37.599 T4 first post-rebuild invocation `capture full --no-edit -c` (name held by stale :1.320)
- 02:10:40.381 exit=0 — TRANSPARENT: stale daemon parked+exited unserved, CLI got NoReply,
  retried, spawned fresh helper PID_B=9149 (clean exe), which served the capture. NO "io failed".
- 02:10:41.401 post: PID_A=7503 GONE, PID_B=9149 alive. VERDICT: PASS.
- The issues.md interactive repro (`flowshot capture` → session-child spawn) was NOT run live:
  task 1's DISPLAY gate is already in the tree, so the child would attempt a real X11 overlay
  window (task 2's in-flight territory) on the in-use machine. The task's criterion "old daemon
  exits and a fresh one serves" is met by the headless run above; the child-spawn ENOENT class
  is proven at mechanism level (readlink + spawn attempt) and the stale daemon now NEVER reaches
  a child spawn (gate rejects before dispatch — asserted by supersession.rs: sink got 0 commands).

### Gates
cargo build/clippy(--all-targets -D warnings)/fmt --check/purity-gate/test --workspace ALL GREEN
(41 suites ok; one transient flake: capture-wayland kwin no_fd_leaks under full-suite parallel
fd pressure — passes standalone ×2, crate untouched by task 4).

### Structural notes / follow-ups
- `Startup::Running` is now `Box<Daemon>`: adding the `superseded` Arc pushed Daemon to 208 B,
  over clippy::large_enum_variant's 200 B budget (lint fired; boxed per clippy's own fix hint;
  all consumers call `daemon.run()` which deref-moves — zero call-site churn).
- overlay/mod.rs color path (`SessionResult::Color`) ALSO sets held(true) with no release hook —
  file is task 2's (MUST NOT touch). Follow-up for task 2/F1: switch it to
  `post::sinks::clipboard_for_run` (bridge is pub(super) inside post; may need a visibility bump
  or a move to execute/mod.rs). On X11 the color path only exists once task 2 lands interactive.
- Tray/shortcut dispatches do NOT pass the bus gate (task scoped the check to D-Bus dispatch);
  a stale daemon exits at its next bus dispatch regardless.
- QA GOTCHA (task 1 interaction): with the DISPLAY gate landed, EVERY daemon spawn on X11 arms
  the first-launch consent prompt (detached `flowshot session --spec ...consent...` child; it
  lingers and matches `pgrep -x flowshot`). For QA runs: set `[telemetry] asked_on_first_launch
  = true` temporarily (backup+restore — done, config restored to false afterwards).
- clipboard.rs hit 253 pure LOC → SIZE_OK marker added (next split: session routing into
  clipboard/session.rs). x11.rs grew to 711 pure (existing SIZE_OK; reassessed per its note:
  the growth is serve-lifecycle logic, not pure helpers — extracting x11/pure.rs mid-phase
  would churn the shipped protocol file for no behavioral gain; deferred to a quiet window).
- pkill caution re-confirmed: used explicit PIDs / `pgrep -x flowshot` + ps cmd filtering
  (the consent child matches `-x flowshot` too — filter on `daemon --auto-spawned` when needed).

## [2026-10-05] Task 4 complete: riders verified

Fresh session (prior one aborted): AUDIT of the inherited task-4 work + the two
LIVE proofs re-run from scratch with attribution-hardened methodology. Evidence:
/tmp/opencode/{proof1a,proof1b,proof2,proof2b}/timeline.log (+ proof1a/daemon.log).

### Audit verdict: wiring complete end-to-end, ZERO gaps found (no code changes)
- daemon.rs: `superseded: Arc<Notify>` created in start() (L183), gate clones it
  (L185-190, production `SupersessionGate::new` snapshots the running binary;
  `supersession_baseline` QA knob for foreign baselines), run()'s select WATCHES
  it (L295 `() = superseded.notified() => ShutdownReason::Superseded`), teardown
  clean (tray -> shortcuts -> close_quietly). Notify permit semantics: a trigger
  before the select starts is still observed.
- bus.rs accept(): stale check runs FIRST (L90), warn + trigger_exit +
  `std::future::pending()` never-reply (L107) -> broker answers NoReply after the
  name release -> CLI retry re-handshakes. tests/supersession.rs proves it against
  a private dbus-daemon (NoReply, reason=Superseded, sink got 0 commands, 0.06s).
- actions x11.rs: serve() claims OWNER_EPOCH BEFORE the owner takes the selection
  (L197); event_loop exits on SelectionClear (L431) / connection death (L419) /
  flush failure (L449); run_owner then calls report_loss_if_latest (L253) which
  fires the hook ONLY if the epoch is still latest -> self-supersede silent
  (setup-failure path exits silent too - no offer to lose).
- post/sinks.rs clipboard_for_run: latch + hold_release_hook (latches, then
  set_clipboard_offer_held(false)); post/mod.rs sets held(true) only when
  `Copied && !lost` (L111-125). state.rs: clipboard_offer is a persistence reason.
- CLI dispatch.rs: OWNER_VANISHED_ERRORS = [NameHasNoOwner, NoReply], single full
  retry; handshake's Acquired branch releases the name, spawn::helper, forwards.
  instance.rs classify_method_error maps NoReply -> OwnerVanished.
- Wayland asymmetry verified by code: for_session_with_loss_hook's Wayland arm
  DISCARDS the hook (clipboard.rs L220-225); ClipboardBackend trait + Wayland
  impl untouched; documented in OfferLossHook + sinks.rs docs. No Wayland change.

### QA methodology findings (MATTER for tasks 6/8/F2 on a shared machine)
1. **Parallel-task interference is REAL and invalidates naive session-bus QA.**
   First proof-1 attempt (02:48, session bus, production shape) was confounded:
   a parallel task's CLI dispatch hit MY daemon (single instance!) at 02:48:19
   spawning a session child (PPID=daemon) that pinned it + touched the activity
   stamp; at 02:58 the X11 CLIPBOARD held a PARALLEL daemon's offer (FlowShot
   4-target signature, bytes differing per screenshot). The X11 selection is
   GLOBAL - any parallel `capture -c` supersedes my offer (fires my hook early);
   my takeover supersedes theirs. Verdicts on the session bus are unattributable
   while other agents run captures.
2. **Fix: private bus via ENV, zero code changes.** `--bus-address` CANNOT ride
   the literal task command: `capture full --no-edit -c` is a MODIFIED form ->
   the lossless `Invoke(argv_tail)` channel (wire.rs table) -> argv_tail carries
   `--bus-address ADDR` -> the daemon's hand-frozen Invoke parser rejects it
   ("unknown Invoke verb" before the verb; "unknown capture argument" after -
   invoke.rs mirrors only the capture grammar). PRE-EXISTING QA-tooling gap, NOT
   task-4, NOT fixed (frozen-subset contract is deliberate). Workaround that
   keeps the literal commands: `export DBUS_SESSION_BUS_ADDRESS=unix:path=...`
   (instance::connect(None) -> zbus Connection::session() honors it; the helper
   inherits env). Attribution oracles: daemon log `ownership acquired window=N`
   (decimal) == readback `owner=0x...` (hex); owner STABLE across the pinned
   phase; serving thread visible in /proc/PID/task/*/comm (flowshot-x11-cl*).
3. **ANSI in redirected daemon logs**: tracing fmt colorizes even to a file -
   grep for `window=`/`reason=` fails on the escape between name and `=`. Use
   NO_COLOR=1 or sed-strip before grepping (my first verdict said FAIL on a
   passing run purely from this).
4. Consent QA gotcha re-confirmed: flipped `[telemetry] asked_on_first_launch`
   to true for the runs (backup /tmp/flowshot-config-task4-backup.toml),
   RESTORED to false afterwards.

### LIVE PROOF 1 - hold-release on SelectionClear (both shapes PASS)
RUN A (manual daemon, env private bus, grace 15s, NO_COLOR RUST_LOG=debug):
- 03:07:31.753 daemon PID 32594 up; 03:07:34.037 `capture full --no-edit -c` exit 0
- 03:07:34.033Z(log) ownership acquired window=37748736; readback owner=0x2400000
  (=37748736) TARGETS=4 [TARGETS, TIMESTAMP, MULTIPLE, image/png] bytes=814466
  IHDR 2880x1620 -> ATTRIBUTION exact
- polls T0+5..+30s: ALIVE (grace 15 exceeded -> pinned by the hold alone)
- 03:08:04.113 T1 takeover `x11_smoke` (exit 0 @ .143); verbatim chain:
  01:08:04.116799Z DEBUG x11 clipboard ownership lost; serving thread exiting
  01:08:04.116890Z INFO  clipboard offer lost to another client; releasing the clipboard-offer hold
  01:08:04.117000Z INFO  idle grace elapsed with no persistence reason - exiting idle_secs=32
  01:08:04.117053Z INFO  daemon shutting down reason=IdleExit        (280us end-to-end)
- 03:08:04.150 poll: daemon GONE (+37ms). VERDICT: PASS.
RUN B (PRODUCTION SHAPE: CLI-handshake auto-spawned helper, null stdio, default
60s grace, pgrep polls every 15s, env private bus):
- 03:10:06.553 T0 capture exit 0 -> helper PID 1937 `daemon --auto-spawned`;
  state: 1 serving thread (comm flowshot-x11-cl*); readback owner=0x2400000
  bytes=814565 IHDR 2880x1620
- polls T0+15/30/45/60/75s: ALIVE srv-threads=1 - pinned PAST the 60s grace
- readback@T1-: owner=0x2400000 UNCHANGED (same bytes) -> the helper's offer
- 03:11:21.671 T1 takeover x11_smoke (exit 0 @ .700)
- 03:11:21.707 poll T1+~0s: helper GONE (7ms after smoke exit) - hold released,
  idle-exit fired; post: no flowshot on the bus, CLIPBOARD has NO OWNER.
  VERDICT: PASS. (Task budget: run B wall time 77s <= 3min.)

### LIVE PROOF 2 - stale-daemon self-check (BOTH gate branches PASS live)
PROOF 2 (deleted-suffix branch; cargo turned the "no-op" rebuild into a REAL
relink - parallel source drift - so the inode swapped):
- before: ino=37687301 mtime=1791162421; 03:13:57.811 T0 capture exit 0 ->
  PID_A=5036, readlink CLEAN, identity == disk (baseline)
- 03:14:06.863 T1 `touch -m` + `cargo build -p flowshot-cli` (Compiling, 8.96s):
  disk ino=37687328 mtime=1791162846; PID_A readlink -> ".../flowshot (deleted)"
- 03:14:06.875 T2 `RUST_LOG=debug ... capture full --no-edit -c`: exit=0,
  elapsed=2.455s (NO hang), stdout EMPTY, stderr = benign zbus name WARN x2 +
  the retry chain: 01:14:06.887179Z DEBUG "forwarding to the running daemon"
  member=Invoke -> 01:14:06.890367Z DEBUG "auto-spawned the helper daemon"
  (NoReply swallowed by the retry predicate between them); "io failed" matches: 0
- POST: PID_A=5036 GONE (exited UNSERVED), PID_B=6087 ALIVE, readlink clean,
  identity ino=37687328 mtime=1791162846 == touched disk binary (fresh baseline);
  readback: NEW offer bytes=814501 -> PID_B served. VERDICT: PASS.
PROOF 2b (identity-mismatch branch ISOLATED - the task's exact "touch moves
mtime -> stale" mechanism, no cargo):
- 03:15:08.745 T0 -> PID_A=6818 baseline ino=37687328 mtime=1791162846;
  T1 `touch -m` ONLY: SAME inode, mtime=1791162908, readlink STAYS CLEAN
- T2: exit=0 elapsed=2.383s, same retry chain (01:15:08.793215Z forwarding ->
  01:15:08.795854Z auto-spawned), io-failed 0; PID_A GONE unserved, PID_B=6930
  identity == touched binary. VERDICT: PASS.

### Gates (03:16-03:17 snapshot; task-2 editing the tree concurrently)
- cargo build --workspace: PASS. cargo clippy --workspace --all-targets
  -D warnings: PASS (exit 0, zero findings). cargo test --workspace: PASS
  (all suites ok, 0 failed; supersession 1/1, stale 9/9, clipboard x11 59/59
  re-verified standalone too). purity-gate.sh: PASS (0 violations).
- cargo fmt --check: **FAIL - NOT task 4**: all 3 diffs in
  execute/overlay/reroute.rs (task 2's in-flight file, mtime 03:13:41, my
  MUST-NOT-EDIT scope; file re-checked 03:18 still unformatted). Task-4 surface
  fmt-clean. Orchestrator: task 2 must `cargo fmt` before its gates; F3 note.
- Self-reversal: config restored (asked_on_first_launch=false), PID_Bs killed,
  private brokers killed, zero flowshot processes remain; clipboard volatile
  (left owner-less - smoke's offer died with its process).

### Follow-ups for the orchestrator
- Invoke-channel vs global flags gap (methodology finding #2): either the CLI
  strips global flags from argv_tail before forwarding, or invoke.rs learns to
  skip them. Pre-existing, QA-only impact, frozen-subset contract is deliberate
  ("never silent drops") - a decision, not a drive-by fix.
- fmt failure in task 2's reroute.rs (above) - cross-task, do not fold into 4.

## [2026-10-05] Task 2: overlay on X11

### A. Live bring-up + transparency decision (plan decision #2: RESOLVED)
- KEEP `with_transparent(true)` - NO customizer change needed. First live
  bring-up (02:48, daemon path): overlay window FULLSCREEN+FOCUSED at
  2880x1620+0+0, `WM_CLASS = "FlowShot", "flowshot"` (task-1 X11 arm proven
  on the overlay too), i3 auto-focused it (`xdotool getwindowfocus` = overlay
  WID) - no grabs, no override-redirect (decision #1's contingency UNUSED).
- Backdrop = the FROZEN DESKTOP, not black: evidence PNG captured headlessly
  from a second invocation while the overlay was up (direct leg is not
  session-gated - task 1's pattern) shows dim layer over live desktop content,
  crosshair, "L Magnifier"/"F Grid" HUD chips. picom composites fine despite
  the inconclusive _NET_WM_CM_S0 query. Evidence:
  .omo/evidence/x11-phase-b/task2/overlay-backdrop-frozen-not-black.png.
- Daemon-path CLI exits 0 at the bus ACCEPTANCE (reply-window contract,
  bus.rs accept() doc) while the overlay still runs - exit code 3 for Esc is
  ONLY observable via one-shot `--no-daemon` (task 6: plan the matrix accordingly).

### B. Interactive matrix - ALL VERIFIED LIVE (evidence bundle + RESULTS.md
### at .omo/evidence/x11-phase-b/task2/)
- Drag-select (xdotool mousemove/mousedown/6x move/mouseup, physical
  (800,400)->(1700,1000)): HUD `400x266+355+177` = EXACT logical (physical
  /2.25), 12-button toolbar per config, grab handles, crosshair.
- Tool action: `key r` (rect tool, toolbar highlight) + `key 5` (digit
  sizing, thick stroke) + drag -> drawn rect PIXEL-EXACT at the drag coords
  (magick %[pixel:p{1000,600}] = srgba(255,0,0,1) = config draw_color;
  interior unfilled). Pointer->logical->shape->render round trip has ZERO
  offset on X11. (look_at position estimates were ~120px off - measure
  pixels, never trust image-model eyeballing for geometry.)
- `key Return` accept: child exited; `[capture].last_region` persisted
  356,178,400,267 (exact logical round); clipboard readback via x11_smoke:
  owner=daemon window, TARGETS 4 incl image/png, IHDR 900x600 (= physical
  selection), export contains the drawn rect at export-space (200,200)-
  (600,500) - red pixels verified, hollow interior. Daemon-owned offer
  outlives the child (hold works on X11, task 4's hook wiring).
- Esc cancel (one-shot --no-daemon): EXIT=3, no children left.
- Color picker (daemon path): pre-shot pixel (100,1610)=srgba(9,13,18,1);
  `flowshot color` -> click -> clipboard text 7 bytes = `#090D12` - EXACT.
  NOTE: the eyedropper samples the STITCHED LOGICAL-RES frame (scale 1.0),
  so pick points must sit in solid-color areas on scale>1 outputs (a 2.25x
  downsample averages the physical block) - (100,1610) i3-bar bg is solid.
- QA gotcha: X REUSES WIDs (39845891 appeared in both run 1 and run A) -
  track sessions by _NET_WM_PID, not WID. `xdotool search --name FlowShot`
  also matches VS Code titles containing the project name - filter on
  WM_CLASS '"flowshot"'.

### C. Reroutes (decision #6) - IMPLEMENTED + live-verified
- Shape: `x11_headless_region(request, ctx)` gained (a) `last_region` branch
  (precedence mirrors wiring's `preselect_for`: last_region WINS over a
  region token; reads `config.capture.last_region` via ctx.load_config();
  Region{i32,i32,u32,u32} -> LogicalRect via lossless f64::from) and (b)
  `--region at-cursor` -> `Target::Screen(ScreenTarget::Cursor)` (the direct
  leg's existing resolve_cursor X11 leg + output_at + first-output fallback -
  no duplicated cursor math). Absent persisted region / malformed tokens ->
  Ok(None) -> overlay child (Wayland parity: interactive wait; the Phase-A
  "honest NoDisplayServer failure" doc was stale post-task-1 and was rewritten).
- Pure decision extracted: `windowless_reroute(request, saved_last_region)`
  - 5 unit tests pin the decision table (conversion incl negative x, absent
  region, precedence, at-cursor, typed/malformed/absent stay with caller).
- LIVE (one-shot --no-daemon, RUST_LOG=info): `capture last --no-edit -o`
  -> log `X11 headless region reroute ... target=Region(Rect { x: 356.0,
  y: 178.0, width: 400.0, height: 267.0 })` -> 900x600 PNG (run A's
  persisted region round-tripped; private XDG_CONFIG_HOME=/tmp/b2/xdg fixture
  after the shared config was clobbered - see E). `capture --region
  at-cursor --no-edit -o` (cursor at 1440,810) -> `target=Screen(Cursor)` +
  `cursor position ladder layer="x11-query-pointer" resolved=true` ->
  2880x1620 PNG. WM_CLASS scan: NO overlay window during either run.
  Earlier `capture last` pass against the then-current config (0,0,1280x720)
  -> 2880x1620: conversion tracks the file exactly.

### D. Structure: overlay/mod.rs SPLIT (250-LOC rule)
- The reroute work pushed mod.rs to 314 pure LOC (DEFECT band; pre-edit was
  ~228, warning band). Split per the module's own documented convention
  ("Splits: wiring ... session ..."): new `overlay/reroute.rs` (118 pure)
  holds x11_headless_region + windowless_reroute + tests; mod.rs back to 202
  pure (WARNING BAND - next editor: consider moving overlay_child/
  prepare_overlay into session.rs). Same seam/owner as decision #6 intended
  (private submodule of overlay/; launcher's `overlay::region_rect_of`
  re-export path untouched).

### E. Parallel-task interference (METHODOLOGY - task 6/F2 MUST read)
- The task-4 completion wave ran live QA on the SAME display/bus/config
  concurrently: my run-1 daemon+child (18446/18677) were killed externally
  between evidence shots (no completion: last_region unchanged, clipboard
  empty, spec reaped by the daemon before it died); a mystery overlay in one
  02:52 frame carrying MY exact drag = the other task's overlay (its toolbar
  showed 4 default-config buttons vs my 12-button config - the tell); the
  shared ~/.config/flowshot/flowshot.toml was restored over my QA state at
  03:30:10 (last_region + asked_on_first_launch both reset mid-matrix).
- Countermeasures that WORKED: (1) binary snapshot `cp target/debug/flowshot
  /tmp/b2/flowshot-qa` - immune to the other task's cargo rebuilds (no
  supersession respawns) and to `pkill -x flowshot` sweeps; (2) private
  `XDG_CONFIG_HOME` fixture for config-dependent runs (paths.rs honors it);
  (3) state verification around every step (pgrep + WM_CLASS scan + config
  grep before AND after). Task 6: serialize live QA or use all three.
- Consent child spawned alongside run A's daemon (the parallel restore had
  set asked_on_first_launch=false): dismissed via `xdotool windowkill`
  (WM_DELETE, task-1 precedent - no answer recorded, prompt re-arms); the
  overlay KEPT focus throughout (dialog never stole it).

### Gates
- cargo build / fmt --check / clippy --workspace --all-targets -D warnings /
  purity-gate (3 crates, 0 violations) / test --workspace: ALL GREEN. One
  kwin no_fd_leaks flake in the first full run (issues.md item 4's known
  fd-pressure flake; 5/5 standalone passes after; crate untouched by task 2 -
  its only modified file desktop.rs is earlier phase work).

### Notes for later tasks
- F1: issues.md item 3 (color path held(true) with no release hook) DEFERRED
  deliberately: the fix needs post/sinks.rs `clipboard_for_run` (pub(super)
  inside post) and task 2's MUST NOT forbade post/* edits (task 4 in flight);
  duplicating the epoch-latched bridge inside overlay/mod.rs would fork
  task 4's single-seam design. Color on X11 works (hex copied, verified);
  residual: after `flowshot color` the daemon pins until manual kill /
  SelectionClear-without-hook. F1: bump the bridge visibility (one line in
  post/sinks.rs) and switch overlay/mod.rs's Color arm to it.
- Task 3 (pins): the transparency question is settled for the overlay but
  pins are genuinely transparent (no opaque backdrop) - with_transparent(true)
  on X11 relies on picom here; a compositor-less X11 session would render
  pin corners opaque/black (decision #2's customizer arm remains the
  documented fallback, now pin-scoped).
- Task 6: evidence bundle seeded at .omo/evidence/x11-phase-b/task2/
  (RESULTS.md = the command/output matrix). Multi-monitor overlay QA stays
  hardware-gated (single panel).
- Task 7 (docs): flowshot-capture-x11/src/lib.rs doc + README "headless
  only" rows now ALSO stale for the reroutes (last/at-cursor work headless
  on X11; interactive ships).

## [2026-10-05] Task 2 (fresh): overlay live

Fresh-session re-run of the FULL task-2 live proof (prior session's evidence
existed but the orchestrator flagged the live part as never done; everything
re-proven from scratch, zero code changes). Evidence bundle + command/output
matrix: .omo/evidence/x11-phase-b/task2-fresh/ (RESULTS.md is the index;
logs run1..run5fg + 9 PNGs + daemon-reroute-mechanism.log).

### Decisions
- TRANSPARENCY (plan decision #2): FINAL = KEEP `with_transparent(true)`.
  Backdrop while overlay up: headless full capture mean=21623/65535 (min
  3084) + visual = dimmed frozen desktop, crosshair, HUD chips — NOT black
  (overlay-backdrop-frozen-not-black.png). picom composites fine. No
  customizer change made.
- FOCUS (decision #1): winit-only path CONFIRMED sufficient on i3 —
  fullscreen overlay auto-focused (getwindowfocus = overlay WID), Esc/keys/
  drag all work; override-redirect contingency stays UNUSED.
- reroute.rs: NOT modified — live proofs found no bug (see the 5a anomaly
  below: QA-state artifact, code faithful).

### Live results (all exit codes measured without pipes)
- First contact (daemon path): window WID 41943043, WM_CLASS
  "FlowShot","flowshot", _NET_WM_STATE FULLSCREEN+FOCUSED+MAXIMIZED_*,
  geometry 2880x1620+0+0, _NET_WM_PID = session child. i3 honors the
  EWMH fullscreen ask (no tiling). Esc → child exits clean.
- Exit 3: one-shot `capture --no-daemon` + Esc → EXIT=3. Daemon path exits
  0 at acceptance (reply-window contract) — task 6 matrix must use
  --no-daemon for cancel-code rows (inherited, re-confirmed).
- Interaction: drag (800,400)→(1700,1000) physical → HUD `400x266+355+177`
  (exact logical ÷2.25 floor) + 13-button toolbar + 8 grab handles;
  `key r` + `key 5` + drag → rect stroke PIXEL-EXACT srgba(255,0,0,1) at
  export-space (200,200)..(600,500), interior hollow; `key Return` →
  last_region persisted 356,178,400,267; clipboard readback owner=daemon
  0x2800000, IHDR 900x600, TARGETS=4 incl image/png.
- Color: pre-shot (100,1610)=srgba(9,13,18,1) → `flowshot color` → click →
  readback text/plain bytes=7 `#090D12` EXACT.
- Reroutes: `capture last` (fixture region 100,100,200,150) → daemon log
  `target=Region(...100/100/200/150)` → 450x338 PNG; `--region at-cursor`
  → `target=Screen(Cursor)` + ladder `x11-query-pointer resolved=true` →
  2880x1620; `--region 500x300` → 1125x675, cursor-centering exact
  (persisted 390,210 = cursor-logical 640,360 − 250,150). All EXIT=0,
  zero overlay windows during runs (0.5s-interval WM_CLASS scan loop).

### i3/X11 quirks + QA methodology (task 6/F2: read before live runs)
- **`capture full`/`screen`/typed-region ALL persist `[capture].last_region`
  (post/mod.rs persist_region — direct-path region memory, by design).**
  Any headless evidence shot taken BETWEEN an interactive selection and a
  `capture last` replay CLOBBERS the persisted region — this fully explains
  my run-5a "anomaly" (`capture last` → 2880x1620: the color pre-shot full
  capture had reset last_region to 0,0,1280x720). Task 6: order the matrix
  so `capture last` immediately follows the selection that defines it, or
  assert against the CURRENT config value, never a remembered one.
- Isolation stack that worked (zero interference): private dbus-daemon at
  unix:path=/tmp/b2fresh/bus via DBUS_SESSION_BUS_ADDRESS + private
  XDG_CONFIG_HOME=/tmp/b2fresh/xdg seeded with asked_on_first_launch=true
  (consent never spawned) + save_last_region=true. Shared ~/.config
  provably untouched (mtime predates session).
- xwininfo is NOT installed here — use xdotool search --class flowshot +
  xprop -id (WM_CLASS/_NET_WM_STATE/_NET_WM_PID) + getwindowgeometry.
  `search --name FlowShot` also matches editor titles (23068676 hit) —
  filter on class. X REUSES WIDs (41943043 in runs 1/2/3/4) — attribute by
  _NET_WM_PID, never WID.
- Watchdog pattern for daemon-path overlays (child is NOT timeout-wrapped
  by the CLI's `timeout`): background `( sleep 40; W=$(xdotool search
  --class flowshot); [ -n "$W" ] && xdotool key Escape ) &` — self-disarms
  when the window is gone, never sends stray keys to user apps.
- Benign noise to expect: zbus name-request WARN per daemon spawn; winit
  "error setting XSETTINGS" WARN on one-shot overlay.
- Gates at session end: build/test/clippy(-D warnings)/fmt --check/
  purity-gate ALL EXIT=0 (purity: 3 crates, 0 violations). fmt debt on
  reroute.rs noted by task 4 was already cleared by the prior session.
- Self-reversal: manual daemon + private dbus killed (both mine), zero
  flowshot procs/windows remain, clipboard left owner-less.

## [2026-10-05] Task 3: pins on X11

Fresh session; started 2026-10-05 18:26 (prior attempt, interrupted — its
/tmp/b3 QA fixture + logs were reused/informed this one), completed
2026-10-06 09:30–12:20. Evidence bundle + full command/output matrix:
.omo/evidence/x11-phase-b/task3-pins/ (RESULTS.md is the index).

### Fixes landed (4, all live-proven; bundle has the before/after logs)
1. **X11 resize path (flowshot-ui, platform-free)** — pins/effects.rs
   `SetWindowSize` pinned min==max hints ONLY (the Hyprland
   compositor-driven trick); on X11 a WM_NORMAL_HINTS update alone
   reconfigures NOTHING (winit 0.30.13 x11/window.rs:1321-1351 =
   XChangeProperty only) → zoom/rotate could not resize the pin (prior
   session's frozen 1472x842 geometry). Fix: also issue
   `Window::request_inner_size` — winit's X11 leg re-pins hints (window is
   non-resizable) AND sends the ConfigureRequest (x11/window.rs:1284-1297);
   Wayland leg = documented no-op for STATEFUL windows (Hyprland sends
   TILED states → stateful → skipped; the verified Hyprland path is
   untouched by construction). Docs updated: shell.rs header, zoom.rs new
   "Resize mechanics on X11" section, event.rs SetWindowSize.
2. **Resize anchor routing (daemon layer)** — X11 size-only
   ConfigureRequests keep the window's TOP-LEFT stationary, so the
   Hyprland-probed `ResizeAnchor::Center` default would mispredict the
   zoom-to-cursor offset by (old−new)/2. New
   `execute/window.rs::session_resize_anchor()` (X11→TopLeft,
   Wayland/none→Center; same detect_session routing as the task-1
   customizer), consumed by pin.rs `PinBehavior`. Proven: zoom
   1472x842→1560x891 with Position UNCHANGED and the pixel under the
   cursor IDENTICAL (srgb(30,30,30) before/after).
3. **Multi-pin gate exemption (daemon)** — the `SESSION_ACTIVE`
   single-window-session gate (session/mod.rs, held for the pin child's
   WHOLE lifetime) rejected a second `flowshot pin` with "a FlowShot
   window session is already active" (CLI EXIT=1) and blocked ANY capture
   while a pin floated. Contradicted the designed multi-pin architecture
   (multi-pin PinRegistry, pins_alive "ANY pin", PinRuntime hosts
   Vec<PinSpec>) and Flameshot parity (allowMultipleGuiInstances gated the
   capture GUI, never pin widgets). Fix: exhaustive match exempts
   `SessionKind::Pin`; Overlay/Launcher/Settings/Consent unchanged.
   Platform-free — unblocks Wayland multi-pin too (not live-verifiable
   here; F3 note). Proven: two pins (1472x842 + 832x632, one daemon, two
   children), independent close.
4. **Synthetic-key chain-close (flowshot-ui, platform-free)** — closing
   pin2 with Esc chain-killed pin1 5 ms later: Esc→pin2 closes → i3
   refocuses pin1 (+2 ms) → winit's focus-in keymap resync
   (handle_pressed_keys) replays the STILL-HELD Escape as a SYNTHETIC
   KeyPress into pin1 → close. BISECT-logged verbatim
   (daemon-run3d-bisect-synthetic-escape.log: is_synthetic:false@pin2 →
   Focused(true)@pin1 → is_synthetic:true Escape@pin1). Hits real users
   (a normal Esc press lasts 80-150 ms). Fix: pins/shell.rs routes only
   `is_synthetic: false` KeyboardInput (+ `key_input` extraction for the
   clippy 100-line cap). Real presses (hardware AND XTest) are never
   synthetic → QA driving unaffected; modifiers still sync via
   ModifiersChanged; Wayland no-op (no focus-resync synthesis).

### QA-ENVIRONMENT defect found first (not a product bug; task 6/F2 MUST check)
**Stuck XTest Escape**: every pin died ~6 ms after spawn on BOTH binaries.
`xinput query-state 5` (Virtual core XTEST keyboard) showed `key[9]=down`
(keycode 9 = Escape) — an interrupted `xdotool key Escape` from the prior
aborted session (killed between the XTest down and up) left the key stuck
SERVER-SIDE; winit's focus-in resync replayed it into every newly focused
window (same mechanism as fix 4). Cleared with `xdotool keyup Escape`.
The user's physical keyboard (slave device 12) was never affected.
**Before ANY live UI QA on this box: `xinput query-state 5` and check for
keys down; also `xinput query-state 4` for stuck pointer buttons.**

### Live results (all exit codes without pipes; SHIPPING binary re-proofs)
- First contact: WM_CLASS "FlowShot Pin","flowshot-pin"; 1472x842 @704,412
  = exact prediction (1440x810 + 32 margin frame); min==max hints;
  undecorated (_MOTIF_WM_HINTS 0x2); i3 AUTO-FLOATS fixed-size windows and
  auto-focuses them. `_NET_WM_STATE` = FOCUSED only — i3 does NOT mirror
  `_NET_WM_STATE_ABOVE` (winit requests it; floating is above tiled anyway
  — recorded quirk, not a bug).
- REQUIRED evidence: `capture full --no-edit -o` while the pin floats →
  pin VISIBLE (crop matches the test image center px EXACT; visual model
  confirms) + a two-pin variant. pin-visible-fullscreen-capture.png /
  two-pins-visible-fullscreen-capture.png.
- Opacity: key 5 → E = 0.5·D + 0.5·U quantitatively (U = the pin's own
  ACCENT-colored drop shadow, paint.rs push_shadow α≈0.13, over the
  desktop — center-px model R EXACT, G/B ±8/255 gamma); key 0 → RMSE(D,F)
  = 0 EXACT restore.
- Drag: press→first-move(+5,+5)→final(+150,+100): Position +145,+95 EXACT
  (= final − first-move; StartDrag defers to first motion by design —
  synthetic-drag math must account for it). i3 honors the
  `_NET_WM_MOVE_RESIZE` grab; XTest mouseup releases it.
- Zoom: click4 ×1 → 1560x891 = exact 1.03², position fixed, cursor pixel
  identical (anchor invariant); click5 ×1 → 1472x842 exact round-trip;
  ×3 clicks (6 steps) → 1751x999 = exact 1.03⁶. **xdotool click 4/5
  commits TWO steps per click** (XTest button + the server's XI 2.1
  button→scroll-valuator conversion; winit processes both) — injection
  artifact, real libinput wheels are valuator-only (winit's own comment).
- Rotate: r → 842x1472 swap (zoomed: scale re-clamps to screen-fit →
  926x1620, the state machine's documented clamp); shift+r → back. i3
  does NOT constrain the overhanging rotated window (bottom 1884 > 1620,
  position kept) — quirk.
- Menu: right-click → Copy/Save/Rotate/Opacity items rendered
  (context-menu.png); Esc closes the MENU first, pin ALIVE; second Esc
  closes the pin (interact.rs precedence ✓).
- Persistence (production shape, auto-spawned, grace 60): pin open 95 s →
  daemon+child ALIVE at every poll incl. T0+65/75/85/95 (PAST grace —
  pins_alive); Esc close → daemon GONE ≤1.9 s later (grace already
  elapsed; unregister released the last reason). Two-pin corroboration:
  daemon exits at last-activity+~60 s after BOTH close. NOTE: a MANUAL
  `flowshot daemon` is Supervised = ALWAYS persists (daemon.rs) —
  idle-exit is AutoSpawned-only (cost me one misread timeline).
- Multi-pin: two pins, one daemon, two children, distinct WIDs/PIDs/
  geometries; close pin2 → pin1+child+daemon ALIVE (chain-close fixed);
  close pin1 → clean; daemon idle-exits on schedule.

### QA methodology (task 6/F2 inherit)
- Isolation stack (task-2 precedent, worked): private dbus-daemon at
  unix:path=/tmp/b3/bus via DBUS_SESSION_BUS_ADDRESS + private
  XDG_CONFIG_HOME=/tmp/b3/xdg (asked_on_first_launch=true) + binary
  snapshot /tmp/b3/flowshot-qa. Shared config mtime verified unchanged.
- **Sleep-based Esc watchdogs are cross-run hazards**: run5's watchdog
  fired mid-run2 and Esc'd its pin (invalidated one persistence run).
  Scope watchdogs to the run's WIDs, or wait for expiry before the next
  pin run.
- **pgrep/pkill -f self-match**: the polling shell's own cmdline contains
  the pattern (pgrep returned the poller; `pkill -f 'flowshot-qa.*daemon'`
  KILLED the invoking tool shell). Attribute with
  `ps -eo pid,cmd | grep -E '^[[:space:]]*[0-9]+ /tmp/b3/flowshot-qa'`;
  kill by explicit PID.
- `import -window root -crop WxH+X+Y +repage f.png` (IM7; NOT
  `magick -window`) captures the composited screen under picom without
  touching the daemon (no activity stamps) — the opacity/zoom frame tool.
- Opacity blend math must model the pin's own accent shadow as the
  underlayer, not the raw desktop.
- A capture command BEFORE the manual daemon auto-spawns its own helper
  which OWNS the bus name — the manual daemon then exits "another daemon
  is already running" and the helper (null stdio, no logs) serves
  everything. For logged runs: start the daemon FIRST.

### Gates (final state) + LOC
- build / fmt --check / clippy --workspace --all-targets -D warnings /
  test --workspace (41/41 ok, 0 failed, no kwin flake) / purity-gate
  (3 crates, 0 violations — the ui changes are winit CORE API only):
  ALL EXIT=0.
- Pure LOC: effects.rs 120, shell.rs 230 (WARNING BAND, pre-existing;
  next editor consider splitting the event translation out), zoom.rs 235
  (warning band, pre-existing), event.rs 49, window.rs 24, pin.rs 166,
  session/mod.rs 220 (warning band, pre-existing).
- Self-reversal: zero flowshot processes/windows; private dbus 14444
  killed; XTest key state clean; shared config untouched; /tmp/b3 fixture
  retained for task 6.
