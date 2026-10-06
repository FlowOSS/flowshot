# Issues — x11-phase-b

## [2026-10-04] Task 4 (riders) — cross-task findings

1. **Broker answers mid-call disconnects with `NoReply`, NOT `NameHasNoOwner`** (empirical,
   tests/supersession.rs against a private dbus-daemon). The CLI's single dispatch retry only
   matched NameHasNoOwner — task 4 extended it to both (dispatch.rs OWNER_VANISHED_ERRORS) and
   instance.rs classify_method_error now maps NoReply → OwnerVanished. F1/F3: review the retry
   safety argument (daemon's 5 s startup-reply window ≪ 25 s bus timeout ⇒ NoReply from a
   FlowShot daemon always means "died mid-call").
2. **Consent prompt now spawns a lingering window-capable child on X11** (task 1's DISPLAY gate
   × the daemon's first-launch consent): every daemon spawn on this machine detached a
   `flowshot session --spec ...consent...` child that matches `pgrep -x flowshot` and (per task 1)
   can open a real dialog on the in-use screen. Task 1/2 QA: disarm via `[telemetry]
   asked_on_first_launch = true` (backup+restore) or expect dialogs + pgrep noise. Task 2: the
   consent dialog itself on X11 is unverified territory.
3. **overlay/mod.rs color path sets held(true) with no release hook** — task 2's file, untouched
   per task-4 MUST NOT. When task 2 lands interactive X11, switch it to the post/sinks.rs
   hold-release bridge (details in learnings.md "Structural notes").
4. **capture-wayland kwin `no_fd_leaks_on_success_or_failure` flakes under full-suite parallel
   run** (fd-pressure sensitive; passes standalone ×2; pre-existing, unrelated crate). Task 5's
   `cargo test --workspace` ×2 should treat a lone kwin fd-leak failure as the known flake and
   re-run standalone before attributing it.
5. **Concurrent live-QA contention on the shared session** (found in task 2,
   matters for task 6/F2): two tasks driving `flowshot` QA on the same X
   display collide through (a) the single bus name (one daemon serves both;
   pkill sweeps kill the other's daemon/children), (b) the shared
   ~/.config/flowshot/flowshot.toml (backup/restore cycles clobber the other
   task's persisted state mid-matrix - task 2's run-A last_region was reset
   at 03:30:10), and (c) cargo rebuilds replacing the binary under the other
   task's running daemon (supersession respawn churn). Task 2's working
   countermeasures: binary snapshot outside target/ (also dodges `pkill -x
   flowshot`), private XDG_CONFIG_HOME fixtures for config-dependent runs,
   state verification (pgrep + WM_CLASS scan + config grep) around every
   step. Task 6/F2: serialize live runs or apply all three.
6. **overlay/mod.rs sits at 202 pure LOC (warning band)** after task 2's
   reroute split (reroute.rs took the X11 headless decision, 118 pure). Next
   editor adding lines there should first move `overlay_child`/
   `prepare_overlay` into session.rs (the module's own Splits convention).
7. **Task 2 update to item 3** (color hold-release): DEFERRED to F1 by the
   task-2 MUST NOT on post/* - the fix is a one-line visibility bump of
   `clipboard_for_run` in post/sinks.rs + switching overlay/mod.rs's Color
   arm to it (duplicating the epoch latch in overlay/ would fork task 4's
   single-seam design). Color on X11 is functionally verified (#090D12 exact
   pick); residual symptom: daemon pins after `flowshot color` until killed.

## [2026-10-05] Task 2 (fresh re-run) — findings

8. **`capture last` QA is state-fragile by design** (no code bug): the
   direct leg persists `[capture].last_region` on EVERY completed capture
   (post/mod.rs persist_region — full/screen/typed-region included). A
   headless evidence shot between an interactive selection and a
   `capture last` replay silently redefines "last". Hit live in run 5a
   (color pre-shot's full capture reset the region to 0,0,1280x720 →
   `capture last` faithfully returned a 2880x1620 capture). Task 6/F2:
   sequence `capture last` immediately after its defining selection, or
   read the config value at assert time. Not a defect — region memory for
   the direct path is documented intent.
9. **No new defects from the fresh live re-run**: overlay focus/fullscreen/
   Esc(=3)/drag/editor/accept/clipboard-hold/color(#090D12 exact)/all three
   reroutes verified green on i3; reroute.rs untouched; transparency stays
   `with_transparent(true)` (backdrop mean=21623/65535, not black).
   Items 3/7 (color-path hold without release hook) remain open for F1 as
   recorded — re-confirmed live: the daemon stayed pinned after
   `flowshot color` until I killed it (mine) during self-reversal.

## [2026-10-06] Task 3 (pins) — findings for F1/F3/task 6

10. **Session-gate Pin exemption is a CROSS-PLATFORM behavior change**
    (session/mod.rs): pins no longer acquire SESSION_ACTIVE, so on
    Wayland too (a) multiple pins can coexist and (b) captures/overlays
    can run while pins float. Both are the DESIGNED semantics (multi-pin
    registry, pins_alive "ANY pin", Flameshot parity — the gate doc cited
    allowMultipleGuiInstances, which gated the capture GUI, never pin
    widgets), and X11-live-proven (two pins + independent close + capture
    with pins up). NOT live-verified on Wayland (no session here) — F3:
    classify as Wayland-regression-risk-by-construction (the overlay/
    launcher/settings/consent arms are untouched; the pin child is an
    independent process either way). F1: review the exhaustive-match shape.
11. **Synthetic-key filter (pins/shell.rs) is X11-motivated but
    platform-free**: `is_synthetic: false` pattern on KeyboardInput. On
    Wayland the field is always false (no focus-resync synthesis) → no-op;
    on X11 it blocks winit's focus-in held-key replay (proven chain-close
    of sibling pins, 5 ms, BISECT-logged). Residual accepted: a user
    HOLDING a digit/r key while a pin gains focus won't get the replay —
    that replay was never user intent for the new window. Overlay/editor
    key handling was NOT touched (MUST-NOT); if the overlay ever shows the
    same artifact (it auto-focuses fullscreen on spawn — a held Esc at
    spawn could cancel it), the same one-line filter applies there (F1).
12. **Stuck XTest keys persist across sessions on this box** (QA-env, not
    code): an interrupted `xdotool key Escape` left keycode 9 DOWN in the
    Virtual core XTEST keyboard for ~14 h; winit's focus-in keymap resync
    then instantly closed every newly focused pin (and would cancel
    overlays at spawn). Cleared via `xdotool keyup Escape`. Task 6/F2
    pre-flight: `xinput query-state 5 | grep down` (keys) +
    `xinput query-state 4` (buttons); symptom signature = window dies
    <10 ms after Focused(true) with zero input sent.
13. **Watchdog cross-run contamination** (QA methodology): a sleep-based
    "Esc any flowshot-pin window" watchdog from run N fires during run N+1
    and kills its pin (happened: invalidated one persistence run —
    run2-final re-run as run2-shipping). Task 6: scope watchdogs to the
    run's own WIDs/PIDs or serialize runs past watchdog expiry.
14. **`flowshot pin` while a SUPERVISED daemon runs**: manual
    `flowshot daemon` always persists (DaemonMode docs) — QA timelines
    must use the auto-spawned shape (or --auto-spawned) for idle-exit
    proofs; a lingering manual daemon is correct behavior, not a leak.
