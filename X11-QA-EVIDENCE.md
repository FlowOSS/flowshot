# X11 QA Evidence — TEMPORARY HANDOFF ARTIFACT

**Status: temporary handoff artifact. Safe to delete after machine-to-machine handoff. Do not treat as permanent documentation.**

This file consolidates the local-only X11 QA evidence bundles into a single committable document. The binary artifacts (PNGs, logs) themselves remain on the source machine under `.omo/evidence/` (gitignored); this document carries the check tables, key observed values, and sha256 fingerprints so the receiving side can verify nothing was altered in transit.

- Generated: 2026-10-06 (source bundles: Phase A 2026-10-04 18:20–19:20 +02:00; Phase B 2026-10-05 01:30 – 2026-10-06 14:20 +02:00)
- Branch: `x11-support` (git HEAD `0bdc99f` at bundle consolidation; binary under test was at HEAD `8961497` per Phase B env dump)
- Source machine: i3/X11 laptop, single eDP-1 2880x1620 @ scale 2.25 (logical 1280x720)
- Source bundles: `.omo/evidence/x11-phase-a/` (10 checks, headless capture) + `.omo/evidence/x11-phase-b/` (20 checks, interactive UI)
- Verdicts: **30/30 PASS, 0 failures**

---

## 1. Environment (merged, deduped; re-measured live both phases, not trusted)

| Fact | Value |
|---|---|
| Session | `XDG_SESSION_TYPE=x11`, `XDG_CURRENT_DESKTOP=i3`, `DISPLAY=:0`, `WAYLAND_DISPLAY` empty |
| Output | eDP-1 connected primary 2880x1620+0+0, 344mm x 194mm; HDMI-1 + DP-1..DP-7 disconnected |
| `RESOURCE_MANAGER` root prop | ABSENT (`xprop -root RESOURCE_MANAGER` → "not found.") ⇒ no Xft.dpi |
| Scale derivation | mm heuristic: 2880/(344/25.4) = 212.65 dpi; /96 = 2.2151 → nearest 0.25 = **2.25** (probe example confirms: logical 1280x720, scale 2.25) |
| Extensions | RANDR 1.5, XFIXES 5.0, MIT-SHM 1.2 (fd-passing fast path true) |
| Root window | screen 0: 2880x1620 px, depth 24, LSBFirst |
| Compositor | picom PID 13180 (uptime 2d07h); `_NET_WM_CM_S0` query returns "not found" yet picom composites correctly (see findings) |
| Phase A binary | `target/debug/flowshot` (fresh; 0 .rs files newer) |
| Phase B binary | `/tmp/b3/flowshot-qa` sha256 `b18a19e314430be4fe2364ee2e0fa0ac9bed016dabbc7bde0e04139bfbae8cd6`, hash-identical to `target/debug/flowshot` @ HEAD `8961497` |
| Tools present | xdotool 4.20260303.1, xinput, i3-msg, python3, dbus-daemon, ImageMagick 7.1.2-32 (compare/convert/import/magick) |
| Tools absent | xwd, xclip, xsel, xdpyinfo, xwininfo (sanctioned fallbacks: `import -window root`, `x11_smoke` example, xdotool+xprop) |
| Phase B isolation | private dbus-daemon (`unix:path=/tmp/b6/bus`) + private `XDG_CONFIG_HOME=/tmp/b6/xdg` (consent disarmed; `[save].actions=["save"]`, path `/tmp/b6/out`); shared `~/.config/flowshot/flowshot.toml` untouched (mtime verified before+after) |
| Phase A config | `~/.config/flowshot/flowshot.toml` actions=["copy"]; last_region saved (0,0 1280x720) |
| Machine state | ACTIVE USE during QA (user workspace switches); pixel comparisons staticity-guarded; XTest device state 0 keys/0 buttons down before+after (Phase B) |

---

## 2. Phase A check table (headless capture bundle, 10/10 PASS)

Source: `.omo/evidence/x11-phase-a/index.txt`. Run 2026-10-04 18:20–19:20 +02:00.

| # | Check | Verdict | Key observed values |
|---|---|---|---|
| 1 | `capture full --no-edit -o` vs independent oracle | PASS | 2880x1620 both readers; 4,665,535/4,665,600 px identical (99.9986%); all 65 diff px = i3 bar clock tick at (2825..2831,9..17); oracle = `import -window root` |
| 2 | `--region 800x600+100+100` geometry + pixel-exactness | PASS | 1800x1350 physical = 800x600 × 2.25; crop of full `--raw` at +225+225 vs region `--raw`: diff_pixels=0, AE=0, first attempt |
| 3 | `capture screen eDP-1` / `screen 0` | PASS | both exit 0, both 2880x1620 |
| 4 | `--raw` stdout PNG | PASS | `file`: PNG 2880x1620 8-bit RGBA; vs back-to-back `-o`: identical except ticking bar clock |
| 5 | `--print-geometry` | PASS | stdout `1280x720+0+0` = logical layout; region form echoes `800x600+100+100`; stderr empty (one-shot in-process) |
| 6 | clipboard `-c` + daemon ownership + INCR + kill | PASS | `-c` exit 0; daemon serves after CLI exit: TARGETS=4 [TARGETS,TIMESTAMP,MULTIPLE,image/png], 1,088,822 B over INCR (4.25× 256KiB cap), IHDR 2880x1620; pkill → readback "NO OWNER"; respawn observed |
| 7 | exit-code matrix | PASS | 0: full/screen/region/raw/copy/delay; 1: bare capture + `last --no-edit` (honest NoDisplayServer); 2: `--region` bogus + legacy `gui`; 4: `DISPLAY=:99` probe failure; 3/5 unreachable on X11 by design; Wayland row hardware-gated |
| 8 | SHM vs forced-plain timing | PASS | shm 38 ms vs plain 80 ms (~2.1×) at 18.6 MB/frame; task-3 range 26-31/72-89 confirmed |
| 9 | `-d 1000` delay path | PASS | exit 0, 2880x1620; 768062 B byte-identical to the clipboard offer served after (same PNG, both actions) |
| 10 | daemon lifecycle | PASS | pinned: daemon w/ clipboard offer alive ~59 min >> 60 s grace (designed); unpinned: fresh daemon spawned 19:17:48 GONE by 19:19:03 (60 s DEFAULT_IDLE_GRACE, lib.rs:118) |

Phase A deviations (none failing): `compare -metric AE` is NOT a pixel count on this IM7 build (calibrated: 10 px @ delta 1 → `0.0392157`); `pkill -f 'flowshot daemon'` self-matches the calling shell (use `pkill -x flowshot`); daemon clipboard hold never cleared by external SelectionClear in Phase A (conservative, unwired by design per state.rs L14-17 — Phase B rider #18 wires + proves it).

---

## 3. Phase B check table (interactive bundle, 20/20 PASS)

Source: `.omo/evidence/x11-phase-b/index.txt`. Consolidated 2026-10-06 13:30–14:20 +02:00 (tasks 1-4 ran 2026-10-05 01:30 – 2026-10-06 12:10).

| # | Check | Verdict | Key observed values |
|---|---|---|---|
| 1 | overlay region select → accept → save/copy | PASS | drag (800,400)→(1700,1000) physical ⇒ HUD `400x266+355+177` exact logical (physical/2.25 floor); 13-button toolbar, 8 grab handles; Return ⇒ `last_region` persisted 356,178,400,267; clipboard owner=0x2800000 (daemon), TARGETS=4, IHDR 900x600; daemon offer outlives child |
| 2 | editor tools smoke (rect + digit sizing) | PASS | `key r` + `key 5` + drag ⇒ stroke pixels (200,200)/(400,200)/(200,350)/(599,499)/(600,500) all srgba(255,0,0,1) = config #FF0000; interior (300,350)/(400,400)=(20,20,20) hollow; pointer→logical→shape→render ZERO offset |
| 3 | overlay transparency decision (plan #2) | PASS — KEEP `with_transparent(true)` | headless full capture WHILE overlay up: mean=21623/65535 (min 3084, max 65535) — decisively NOT black; dimmed frozen desktop + crosshair + HUD chips; picom composites it; no customizer change |
| 4 | Esc cancel → exit 3 (overlay, one-shot) | PASS | one-shot `capture --no-daemon` + Esc ⇒ EXIT=3, no children left; daemon path exits 0 at bus acceptance (reply-window contract) ⇒ cancel code only observable one-shot |
| 5 | color picker (`flowshot color`) | PASS | pre-shot pixel (100,1610)=srgba(9,13,18,1) ⇒ click ⇒ clipboard text/plain bytes=7 payload `#090D12` EXACT |
| 6 | `capture last` headless reroute (no overlay) | PASS | fixture last_region 100,100,200,150 ⇒ daemon log `X11 headless region reroute... target=Region(Rect{x:100,y:100,w:200,h:150})` ⇒ 450x338 PNG (200x150 @2.25 ceil); WM_CLASS scan: ZERO overlay windows |
| 7 | `--region at-cursor` headless reroute | PASS | cursor 1440,810 ⇒ log `target=Screen(Cursor)` + `cursor position ladder layer="x11-query-pointer" resolved=true` ⇒ 2880x1620; one-shot + daemon legs both EXIT=0 |
| 8 | `--region WxH` (offset-less) cursor-centering | PASS | `--region 500x300` ⇒ 1125x675 (500x300 @2.25); persisted 390,210 = cursor-logical(640,360) − half-size(250,150) exactly |
| 9 | pin lifecycle (spawn/drag/zoom/rotate/opacity/close) | PASS | WM_CLASS "FlowShot Pin","flowshot-pin"; 1472x842 @704,412 exact (1440x810 + 32 margin); min==max hints, undecorated, i3 auto-floats+focuses; drag +145,+95 EXACT; zoom click4 ⇒ 1560x891 = 1.03² exact, Position UNCHANGED (TopLeft anchor), cursor pixel identical; rotate ⇒ 842x1472; opacity key5 ⇒ E=0.5·D+0.5·U quantitative, key0 ⇒ RMSE(D,F)=0 EXACT restore; right-click menu rendered; Esc precedence menu-then-pin |
| 10 | multi-pin + independent close | PASS | two pins (1472x842 + 832x632), ONE daemon + TWO session children, distinct WIDs/PIDs; close pin2 ⇒ pin1+child+daemon ALIVE (synthetic-key chain-close FIXED; pre-fix bisect log proves the 5 ms chain-kill) |
| 11 | pin persistence (pins_alive past idle grace) | PASS | production shape (auto-spawn, 60 s grace): pin open 95 s ⇒ daemon+child ALIVE at T0+65/75/85/95 (past grace, pins_alive reason); Esc close ⇒ daemon GONE ≤1.9 s later |
| 12 | settings window opens on X11 | PASS | headless capture while open: settings window VISIBLE (General/Interface/Filename/Editor/Shortcuts tabs, indigo accent); WM_CLASS "FlowShot Settings","flowshot-settings"; i3 auto-focus |
| 13 | consent dialog on X11 | PASS (task-1 artifact) | same task-1 PNG shows "Help improve FlowShot?" consent dialog overlaying settings (WM_CLASS "Help improve FlowShot?","flowshot-consent"); dismissed via WM_DELETE ⇒ re-arms; not re-triggered in task 6 |
| 14 | launcher dialog opens on X11 | PASS | window in ~0.25 s; WM_CLASS "Capture Launcher","flowshot-launcher"; _NET_WM_STATE_FOCUSED; geometry 900x396 @990,635 = exact (400x176 logical ×2.25); i3 auto-floats; combo "Manual region", geometry field auto-focused, Capture DISABLED until valid |
| 15 | launcher lists X11-probe outputs | PASS | Shift+Tab + Space ⇒ ComboBox popup lists "Manual region" AND "Screen 0: eDP-1" (probe_outputs_x11, launcher/mod.rs:198; label via model.rs monitor_label) |
| 16 | launcher submitted capture works | PASS | typed `400x300+100+100` ⇒ Capture ENABLED ⇒ Return ⇒ one-shot child captured DIRECTLY in-process ⇒ saved PNG **900x675** (400x300 ×2.25 exact), CLI EXIT=0, zero windows left |
| 17 | launcher cancel → exit 3 | PASS | single Esc ⇒ EXIT=3; popup-open + Esc+Esc ⇒ EXIT=3 (first Esc closes popup AND fires dialog Cancel, ui.rs:43); reproduced EXIT=3 in THREE clean runs incl. the interleaved-capture confound |
| 18 | rider: clipboard hold-release on SelectionClear | PASS | RUN A (manual daemon, 15 s grace): owner=0x2400000=37748736 ATTRIBUTION exact, TARGETS=4, IHDR 2880x1620; pinned T0+5..+30 s; takeover ⇒ verbatim chain `ownership lost` → `releasing the clipboard-offer hold` → `idle grace elapsed` → `IdleExit` (280 µs), GONE +37 ms. RUN B (production auto-spawn, 60 s grace): pinned T0+15..+75 s, takeover ⇒ GONE +7 ms, CLIPBOARD owner-less. Wayland asymmetry documented (wl-clipboard-rs has no replaced callback ⇒ conservative never-clear kept there) |
| 19 | rider: stale-daemon self-check (deleted-suffix) | PASS | T0 ⇒ PID_A=5036 (ino=37687301); rebuild ⇒ readlink `(deleted)`, disk ino=37687328; T2 exit=0 elapsed=2.455 s (<< 25 s bus timeout), retry chain `forwarding` → `auto-spawned` (NoReply swallowed); io-failed=0; PID_A GONE unserved, PID_B=6087 ALIVE fresh baseline, new offer bytes=814501 |
| 20 | rider: stale-daemon self-check (identity-mismatch) | PASS | `touch -m` ONLY (same inode 37687328, mtime moved, readlink clean) ⇒ staleness via identity compare alone; T2 exit=0 elapsed=2.383 s, same retry chain, io-failed=0; PID_A=6818 GONE unserved, PID_B=6930 identity == touched binary |

---

## 4. Notable findings / deviations

### The 3 pin bugs task 3 found and fixed (all verified live post-fix)

1. **`request_inner_size` missing on X11** (`flowshot-ui pins/effects.rs`): on X11 a WM_NORMAL_HINTS update alone reconfigures nothing (winit 0.30.13 x11/window.rs:1321-1351 = XChangeProperty only); the client ConfigureRequest is the resize path. `SetWindowSize` now also issues `Window::request_inner_size`. Before: zoom/rotate could not resize the pin on X11 (geometry frozen at 1472x842). Wayland side is the documented no-op for stateful windows.
2. **`ResizeAnchor` platform routing** (`flowshot-daemon execute/window.rs` + `execute/pin.rs`): new `session_resize_anchor()` — X11 → `TopLeft` (size-only ConfigureRequest keeps top-left stationary; verified: zoom left Position 704,412 UNCHANGED), Wayland/none → `Center` (Hyprland live-probed default). With Center on X11, zoom-to-cursor would mispredict by (old−new)/2.
3. **Session-gate exemption for pins** (`flowshot-daemon execute/session/mod.rs`): `SessionKind::Pin` exempted from the `SESSION_ACTIVE` single-window-session gate (exhaustive match; Overlay/Launcher/Settings/Consent unchanged). The gate held for a pin's whole lifetime, so a second `flowshot pin` failed "a FlowShot window session is already active" (CLI EXIT=1) and any new capture was blocked while a pin floated — contradicting the designed multi-pin architecture and Flameshot parity. Platform-free; affects Wayland too.

Plus a 4th X11-specific fix in the same task: **synthetic `KeyboardInput` events no longer routed** in `flowshot-ui pins/shell.rs` — winit X11 replays still-held keys as SYNTHETIC KeyPresses on focus-in; proven chain-close race: Esc→pin2 closes (.728146) → i3 refocuses pin1 (.730369) → pin1 receives synthetic Escape (.732745, is_synthetic:true) → pin1 closes (.732847), 5 ms end to end. Hits real users (an Esc press lasts 80-150 ms). Real presses are never synthetic; no-op on Wayland.

### CM_S0 compositor-oracle finding

`xprop -root _NET_WM_CM_S0` returns "not found" on this box in BOTH phases, yet picom (PID 13180) composites the transparent overlay and pins correctly (task-2 backdrop evidence: mean=21623/65535 not black; task-3 pin shadow/opacity blends exact). **The CM-owner probe is not a reliable compositor-presence oracle on this box.**

### Multi-monitor hardware gate

Single-panel machine (eDP-1 only; HDMI-1 + DP-1..DP-7 disconnected). Multi-monitor overlay spanning is recorded as hardware-gated, not claimed. The Wayland-row of the Phase A exit-code matrix is likewise hardware-gated here.

### Transient (non-reproducible, not a defect)

First exploratory launcher run (l1-exploratory.log) recorded `CLI exit=0` after a popup+double-Esc with no save — a cancel-class path that should yield 3. Did NOT reproduce in three clean re-runs (l3 single-Esc, l4 popup-clean, l5 popup WITH the exact interleaved-capture confound), all EXIT=3. Classified as a one-off input/timing race in the exploratory harness, not a launcher defect. No code change; recorded for honesty per docs/verification.md.

### QA-environment defect (not a product bug)

Stuck XTest Escape (keycode 9 down in Virtual core XTEST keyboard) from an interrupted prior-session xdotool left every newly focused pin dying ~6 ms after spawn (winit focus-in key resync replays held keys). Cleared via `xdotool keyup Escape`. Pre-flight `xinput query-state 5/4` is now mandatory before live UI QA on this box.

### Wayland-only leftovers referenced

- `capture last` on X11: headless reroute only (no overlay); the overlay path for `last` is the documented Phase A gap (exit 1, honest NoDisplayServer).
- Pin Esc-close/double-click-close parity: double-click-close not driven live (deferred-drag design preserves it; unit-tested in pins/tests.rs).
- Wayland clipboard hold: conservative never-clear kept (wl-clipboard-rs lacks a replaced callback) — deliberate asymmetry vs the X11 hold-release wired in rider #18.
- The session-gate exemption (fix #3) affects Wayland too but was not live-verifiable on this machine.

---

## 5. Artifact manifest

Binary artifacts remain on the source machine under `.omo/evidence/` (gitignored). sha256 computed with `sha256sum`; PNG dimensions via ImageMagick `identify`. Paths relative to `.omo/evidence/`.

### x11-phase-a/ (21 files)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| 00-environment.txt | 4629 | 3eeb310b5d047ed75bfa9a21821e04e8b626d41f670969252e634bda87830b3f | |
| 01-full-capture.txt | 4843 | 42c58cfb6f2c8dcc5370da0f3fa153f9d7935075ad86561e3897b6e300e8a69c | |
| 01-full.png | 786013 | d4146ca507a7510f5e73c4907112b6e706883efc171b2f0a71348f5d2a3d1dd6 | 2880x1620 |
| 01-oracle-root.png | 327956 | d5fedf3de4d3fc3a71ba8268fc7de879b350d799ebf4d7408d30274f4c2edfe0 | 2880x1620 |
| 02-full-crop-225-225-1800x1350.png | 77249 | 2d9217e2f45f64d52145090db7b0bc831657a64562c4d232af8e7af0213984fc | 1800x1350 |
| 02-region-1800x1350.png | 435568 | 74c1ca7dd154c461deb3b4f1886358359bd43bb062e04de0fc8287068fc8505b | 1800x1350 |
| 02-region-geometry.txt | 2906 | d19d39ef22f3d3dc2481e42c2e9bde68597c879b367e7c25431386bf47a2bd54 | |
| 02-region-raw.png | 435568 | 74c1ca7dd154c461deb3b4f1886358359bd43bb062e04de0fc8287068fc8505b | 1800x1350 |
| 03-screen-eDP-1.png | 739835 | 3803017f10471f0b352032dd1342190d02988e2de8c6f78bc8665d16ffc99a0d | 2880x1620 |
| 03-screen-target.txt | 1030 | 3bac44041bdf462e5e37c5830db444024204da636f7aa8b6a1b36a5736dc47ad | |
| 04-raw-stdout.png | 975424 | 38b03dad9a6531ed7f4816b55999c8bc3353429e712b86b4cb6ab3b663e89dfb | 2880x1620 |
| 04-raw-stdout.txt | 2102 | 8afcef44ad324b2d9909d69191f12dcb5af6217eaa70ea895f60a6844e425797 | |
| 05-print-geometry.txt | 893 | ea6eb91583fddc980981f7b310584aab82b87a404f2b28d7bdf86b2c1bf25133 | |
| 06-clipboard-daemon-ownership.txt | 4524 | 880ab235914bcf16874f0338e1a0ea1b03cd4d56e3cee976f2e33b419c2c4da9 | |
| 06-clip-readback.png | 1088822 | f2c86dc5f23669f16f649af7b6e5410491ad1cb6e37b0bfe0b60902280c49c45 | 2880x1620 |
| 07-exit-code-matrix.txt | 6134 | e5dbdc30812031aa3a66e61ac7c897220603af3de1094a4a9cdc9e82788633ab | |
| 08-shm-vs-plain-timing.txt | 2298 | f0b43aa102cdbde525caf2d9aedde101e50c994da74d4928f464f683c90d0e6c | |
| 09-delay-1000ms.png | 768062 | 09cb399b3da9a4804c6ad58dbc908b01d02fff7e9cb68a03ad6ebad59c8015f5 | 2880x1620 |
| 09-delay.txt | 773 | e0744d7ea6425a201d990406c76bcfc2e9e75d23054f868aa08b6d3ce24aab5d | |
| 10-daemon-lifecycle.txt | 2578 | 435ad8edefe14415f24f55d120b28113baed266e990f4b72269383f95e8b4e7c | |
| index.txt | 7840 | 6fd2f27c76c93e3afc6c7180319829bbaed1dda9145726512cc0747fc01389b8 | |

Note: 02-region-1800x1350.png and 02-region-raw.png are byte-identical (same sha256) — the region capture equals the raw region bytes exactly, which is the point of check 2.

### x11-phase-b/ root (3 files)

| File | Bytes | sha256 |
|---|---|---|
| 00-environment.txt | 2996 | 8aa85f542374fc1dc39e3b4b9eed56f2b2ca3a650c37c449c1c04e95267f3a91 |
| index.txt | 19236 | 05d1f416a757e5c4fa73face2cfc149a8009c108c4ff3ed1ffef74c3d3d1ad9f |
| self-reversal.log | 2297 | 104961d424ebe343e3f1546c45c97c585b0c237cb84358d5e5bafb6c17d9e9b1 |

### x11-phase-b/task1-settings/ (2 files)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| settings-cli.log | 302 | 95eee92678aac1eac1498a799465c086d0249f6f21f061c6f2f88f65c42ac981 | |
| settings-window-visible.png | 801562 | 75d03d06da04514305e5ee017ca62bdab68bef228ba6fdc4cbdf66a1ddf9a006 | 2880x1620 |

### x11-phase-b/task2/ (13 files — EARLIER interrupted session, SUPERSEDED by task2-fresh; retained for history)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| RESULTS.md | 4565 | c53b2a577a599b7efe106c141cb693c6b82cde79c8e2484e00ab7a966709cb14 | |
| capture1.exit | 7 | 418a5c17f33c70e99b0cc0a07fce69191489cfedc94164bfa903785777c5bd4b | |
| clipboard-export-900x600.png | 15052 | 055f88fc791f331537713b18dbb804447bac62e4be238bd26c282ff6234e84bf | 900x600 |
| color-pick-hex.txt | 7 | ef27a44b89a25cd1954a0426677a957dd1e13b0c2dde940fdc3d9c3c680e2a59 | |
| overlay-backdrop-frozen-not-black.png | 678948 | 295a6c1bc24d0c546694c1f0a427130e03d70ed78ccf449ccfff9386ec2ad314 | 2880x1620 |
| rect-tool-drawn-digit5-sizing.png | 776684 | 7797191bde2b6eddf903b292a97ed744b87b981b765a3c94e09fcc790af6c517 | 2880x1620 |
| reroute-at-cursor-2880x1620.png | 806559 | 89318c9b1f938e92dcc6f97915916de707061ba33510ca124a22dd9b36219f65 | 2880x1620 |
| reroute-last-900x600.png | 13828 | 9185941d31e016a9d6845fd6a5c40d33e0cee854cb8c56870784dabd66fa8bbe | 900x600 |
| reroute-last-fullscreen-region-2880x1620.png | 806539 | 8a69b165c5fe3e0661230ec287d60e0040e9a840448bc6374d7295a0b2d9cdce | 2880x1620 |
| runA.exit | 7 | 418a5c17f33c70e99b0cc0a07fce69191489cfedc94164bfa903785777c5bd4b | |
| runB.exit | 7 | b4d0df87aa0481b9b2fb23326f5dbbc9a0fe59fe78852f937b0b37e73a0e2dfa | |
| runC.exit | 7 | 418a5c17f33c70e99b0cc0a07fce69191489cfedc94164bfa903785777c5bd4b | |
| selection-toolbar-hud-400x266.png | 778527 | c9738b9e4e1216a95c8560e89b729fff3fa6e33c922cfa4a35a72ea81564744a | 2880x1620 |

### x11-phase-b/task2-fresh/ (24 files — AUTHORITATIVE overlay/editor/color/reroute re-run)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| RESULTS.md | 6901 | df6867ae1f7b22edf983f2493824b1541f7fbb711c0103627e8b057477281bd4 | |
| clipboard-export-900x600.png | 50408 | 9a7a691c4db5460b18eec84605ec1848ba0a51cc1b1c196a7176a67613731c95 | 900x600 |
| color-pick-hex.txt | 7 | ef27a44b89a25cd1954a0426677a957dd1e13b0c2dde940fdc3d9c3c680e2a59 | |
| daemon-reroute-mechanism.log | 3289 | 0b936ac433c5c64140f6a4f57c5fcc32b63dce83633eb4da6892cd6de968fa41 | |
| overlay-backdrop-frozen-not-black.png | 663900 | 91cdcb1cafd6ef97562929ffe3b329952fb7057895526613d1129839eaa39e98 | 2880x1620 |
| rect-tool-drawn-digit5.png | 702107 | ac899c4f889c10cfeec9c57393a620a3f31051dfb8af378664e60430ed91df7c | 2880x1620 |
| reroute-500x300-1125x675.png | 49044 | ac747ea4bbd5ccf4796f6a7d3364010babaa36efceff096799b977d047cafce4 | 1125x675 |
| reroute-at-cursor-2880x1620.png | 743786 | b12af9403cd849e1cba859db35c5216a5384327149d0b1847c328897f66e9522 | 2880x1620 |
| reroute-at-cursor-oneshot-2880x1620.png | 756310 | 52df9065f989514744573a43a5bbed9f063da45483985f5306a21106a48e9bac | 2880x1620 |
| reroute-last-450x338.png | 76367 | 32b6a10e805bbcd3d9b0731c31092a34f294fd02df9fd473f00a4b4ecc6aa19a | 450x338 |
| reroute-last-fullscreen-region-2880x1620.png | 732598 | 46bec7d2e0da21d32ac3fdf2645c73a56b233b61edf90e37580195daaba0b654 | 2880x1620 |
| reroute-last-oneshot-1125x675.png | 49044 | ac747ea4bbd5ccf4796f6a7d3364010babaa36efceff096799b977d047cafce4 | 1125x675 |
| run1-first-contact.log | 1165 | ffc501f340ccdb483ca87276a519584f15d4a145418975accedde71baa68d4db | |
| run1b-transparency-esc.log | 385 | 470a43ad19873d8ef01b4e8a8ed2da01a7600006715cbfb61d27dfefb819a84c | |
| run2-esc-exit3.log | 398 | 9910d44cdf33fa8daedda9eabcb3783c8248042679678b556e8d4b5ad9133c13 | |
| run3-interaction.log | 2093 | a03c998d492e6d2001f2eff6732bbaae580ce3021cd263a91b45810621296187 | |
| run3-pixels.log | 450 | 83cb1268d9534f72f804f2302b639e2c7e890094cf83e530183875e06acae0bc | |
| run4-color.log | 835 | 42260f17ee9e5c968f6ba84768ffc5d139dd8fa8a969751e6e4b6bdb027c348a | |
| run5-reroutes.log | 890 | 5c21efd40d085fa20a5e4c32169a6319f13b836002673330abb16d1a6867e367 | |
| run5d-last-oneshot.log | 855 | 1315d31cbbaf4c3081ce75cb9cd306b6bcbe4acd5c62e16f5d8df6cbd2f090a1 | |
| run5e-last-daemon.log | 1001 | 56d31c51da8f853d3847ef8d0a4dd02efaec9b6adc0c637f2454a02e94f08ef5 | |
| run5fg-reroute-proof.log | 1434 | e8cae33347675c0cfaac0bedcc4c09bac49a57bc8dd899796b629bda4d699591 | |
| selection-toolbar-hud.png | 705520 | 99182b421a2416e98ddff2f98221b614d6d5b581afd97b5ebc434d018b3ba87c | 2880x1620 |
| self-reversal.log | 348 | 5aabe3ce9989e3f6548a44a4efe80c128b22db0c3ce64b694b0948da04857139 | |

Note: reroute-500x300-1125x675.png and reroute-last-oneshot-1125x675.png share sha256 (byte-identical 1125x675 captures of the same region). color-pick-hex.txt in task2/ and task2-fresh/ share sha256 (both contain `#090D12`).

### x11-phase-b/task3-pins/ (34 files)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| RESULTS.md | 14428 | eb4e3f9afc4c0a3950d388199b5e63433c060d9af17528409b9bac9f92d0a6bb | |
| context-menu.png | 8454 | 95e450d969526eec78f82262df528f20af39aa89b36ceeac457434f0b24ad44a | 500x400 |
| daemon-run1-supervised-exits-name-taken.log | 110 | 01da80aea26bc87adb99a4647f5ef78edf0ee4c367fd346ecb273ed7e9de66c6 | |
| daemon-run3c-chainclose.log | 7574 | ca38a845b7c2c598a3981e90d22281e28d477bab08408995e7bc4b09d4172c89 | |
| daemon-run3d-bisect-synthetic-escape.log | 8363 | 433c758fc8cb68f1000e4b8fd1c54615456162fda0c0cf4408559f814fc31d34 | |
| diagnosis-stuck-escape-daemon.log | 4245 | 7228e2b01b9db259840b6d651b097f91b75269657579bfffba67941c46326f10 | |
| diagnosis-stuck-xtest-escape.txt | 1620 | 1fc1ce7e6e9bde3f38ec62d2bf416d539ea70c0bea0254cbe14d48fbcfa5868f | |
| final-opacity-0.5-E.png | 190 | a8d66c2a0004d3ade2df739014305e5b5d5eeaf38f79f893406992c0d2f4e8c9 | 300x300 |
| final-opacity-1.0-D.png | 537 | c0d0560ac23b223eca2c8bd7fcc26cef099b1c3901e74158f1a67ba26da5c851 | 300x300 |
| final-opacity-restored-F.png | 537 | c0d0560ac23b223eca2c8bd7fcc26cef099b1c3901e74158f1a67ba26da5c851 | 300x300 |
| opacity-0.5-E.png | 11135 | 6aca7417f772f26e31504d419c12306cc6d4d9637c1de19a010042fd4ece67c6 | 300x300 |
| opacity-1.0-D.png | 537 | c0d0560ac23b223eca2c8bd7fcc26cef099b1c3901e74158f1a67ba26da5c851 | 300x300 |
| opacity-background.png | 2047 | 8efa262f1edb0bb04c8022dbf43bfe040e9822f09ea82b9fa5990e82d6950df9 | 300x300 |
| opacity-restored-F.png | 537 | c0d0560ac23b223eca2c8bd7fcc26cef099b1c3901e74158f1a67ba26da5c851 | 300x300 |
| pin2-gate-error-cli.log | 479 | c2a8d2426604edc28e2d1b9390cd0c379ed408417b29ca89d348cde4e6d30c69 | |
| pin-visible-fullscreen-capture.png | 594913 | dd7eccfbac711c70850a591a3b82fd0128480e4689771f7c0fa924f9e39a534a | 2880x1620 |
| run1-first-contact.log | 1368 | 32b71298ade6bb4c290ba646e18bf4badf077ead61b33f67efb85c4f016db9ba | |
| run1b-interactions.log | 2961 | f9bd01b4c5d9e36f9812e322280aea5b4eb11e1aa98ac4b433d8b783e4cd71d7 | |
| run1c-opacity-zoom-menu.log | 1489 | 9be64a77883f419ba33cf4e3806b6ea3ddd003c348314e20a19b3d997e0ce882 | |
| run2-persistence-prefix-binary.log | 1039 | fcad62bb7eea5b94f7ea0018a1c60c4ac36960457cc39eea03ab2e1a4f339d86 | |
| run2-persistence-shipping.log | 1039 | 566ef5de507f32288b32e70712ed03920642a23e0a991e04e952fb16c19a6fbf | |
| run3-multipin-gate-blocked.log | 2249 | d2bd6b7c668ea5cf36a0f10b48d3b52ef3b1de4ef48dd5ed0bbd128bf291ee8d | |
| run3b-multipin-chainclose-anomaly.log | 2273 | 3ec11360ef1c28fe5083d4937168dce1ac99b466d442110fd36a5f1ac1c16b53 | |
| run3c-chainclose-logged.log | 3775 | 53118c4d8d76ab9f818fdd0cce2ba8445057b96e3449de2bb358a5068ec2a5bf | |
| run3d-chainclose-bisect.log | 5532 | 32f7c0fd7940fc8aa0f6302fec3b10c191f06fd742bc5d15a80ef123b14c044d | |
| run4-multipin-fixed.log | 2254 | 1a067bbdcd35fecd6fac9c07466d9ad71dc6cd438a3efb4b3e3f6b0b5bb497f3 | |
| run4-pin1-cli.log | 269 | 7e66f6ee85573917e950a50b924169a7b4b64508f0c16f104e9d9b0fff0d57d8 | |
| run4-pin2-cli.log | 269 | 4909f7033f020e44a98642bfe395a1639fdd763be58cfe6599cd151062a4e3d4 | |
| run5-final-binary-interactions.log | 983 | 37d2ebd4834aae177a8b3ff38738861c83d548e9e65a358616517a29d4d2dc31 | |
| run6-postrefactor-key-smoke.log | 357 | 89ea3ad6dcb982605dfd1c14746a8b4373958227a392fadda5e4ade53550d3fa | |
| test-image-1440x810.png | 70154 | 37ecc6a44114a6910e4244a4034063cddc286f37d6fb2327270b9d01d7130b41 | 1440x810 |
| test-image2-800x600.png | 7199 | c0fa8835447e7a498cd07e410de2a98760a10c1568ba4dee3fe7e3e9af8729b4 | 800x600 |
| two-pins-visible-fullscreen-capture.png | 583055 | 3dd9b9c1153ce26aa06713858e000b35925db296918f9fac8b2a586eb5b397b8 | 2880x1620 |
| zoomed-1.0609.png | 557 | 5bca2eb3f8de535a486f82fa9cc1396c6c04cb4f00e507498069921447ded22a | 400x300 |

Note: the four 537-byte 300x300 PNGs (final-opacity-1.0-D, final-opacity-restored-F, opacity-1.0-D, opacity-restored-F) share sha256 — the quantitative RMSE(D,F)=0 exact-restore claim lives in the same bytes.

### x11-phase-b/task4-riders/ (4 proof dirs, 20 files)

| File | Bytes | sha256 |
|---|---|---|
| proof1a-hold-release-manual/bus-addr | 74 | 1461c12bf61e1530d3203340f7e179408e4d3dfc885cc31cc20212e4f7d1034b |
| proof1a-hold-release-manual/bus-err | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof1a-hold-release-manual/bus.xml | 262 | 44048e868c9456d9cda358ad0dccf5c6edcb66ce16df83393468e8654acaa6ad |
| proof1a-hold-release-manual/daemon-clean.log | 4978 | 17033999e4686dcdb06709046fef3ef9729f1ab972816c787daad39094e914d1 |
| proof1a-hold-release-manual/daemon.log | 4978 | 17033999e4686dcdb06709046fef3ef9729f1ab972816c787daad39094e914d1 |
| proof1a-hold-release-manual/timeline.log | 2960 | 72aa45d5c5fd8cf4a87e67a74a3f13bdcd1c8f5b14dadeb46077dd6f9a4488ef |
| proof1b-hold-release-production/bus-addr | 74 | 2bfb0fc8730f15c6442044fd27bc46055974483f6affee6a08ff67dc453bf87e |
| proof1b-hold-release-production/bus-err | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof1b-hold-release-production/bus.xml | 262 | 27d7dfa502b301419b066c089caa254925741b1d4e389e16de4e1bbd617ef013 |
| proof1b-hold-release-production/timeline.log | 2618 | 070868b5e44112de404681ff4a52f702b7a21d94f16cdf1e577339f2ffd99bee |
| proof2-stale-deleted-suffix/bus-addr | 73 | ebc66d731944b1c059308522d797c78e9d341f1dbe110a1a7a007b4ec512e398 |
| proof2-stale-deleted-suffix/bus-err | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof2-stale-deleted-suffix/bus.xml | 261 | d173d70b7cb63d016cf1126eb05c33c26a2bb361b4ff7ed17fa3c6e675edd29f |
| proof2-stale-deleted-suffix/t2-stderr | 909 | 87e3057adaa110ddf483aa5979afd16b53545298d3849f61b8e571a157bbc0a3 |
| proof2-stale-deleted-suffix/t2-stdout | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof2-stale-deleted-suffix/timeline.log | 4173 | 4f2250242bc5b72d8c5920f1a72fc8d026302c54eea9702161302a0acfa528e9 |
| proof2b-stale-identity-mismatch/bus-addr | 74 | 45e519674e46f4ef4157d38f523ee22d018e09ea0a4af06d7968ceea5941bebf |
| proof2b-stale-identity-mismatch/bus-err | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof2b-stale-identity-mismatch/bus.xml | 262 | 41af7d33c02e64bc8af62e42c4f456325f7b085a316ce16ab284bc9d41b52018 |
| proof2b-stale-identity-mismatch/t2-stderr | 909 | 79e5167be4d055e17786d5282810c4f679fad76da84f2f95e2fd2b96fae67f9b |
| proof2b-stale-identity-mismatch/t2-stdout | 0 | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 |
| proof2b-stale-identity-mismatch/timeline.log | 3705 | 58d32a7f24be52534a6e34a5be375998d7f7357cf6882432dd20efca7365ca51 |

Key timeline lines (hold-release, proof1a RUN A, verbatim from timeline.log, ANSI-stripped):

```
2026-10-05T01:07:34.033749Z DEBUG flowshot_actions::clipboard::x11: x11 clipboard ownership acquired window=37748736
2026-10-05T01:08:04.116799Z DEBUG flowshot_actions::clipboard::x11: x11 clipboard ownership lost; serving thread exiting
2026-10-05T01:08:04.116890Z  INFO flowshot_daemon::execute::post::sinks: clipboard offer lost to another client; releasing the clipboard-offer hold
2026-10-05T01:08:04.117000Z  INFO flowshot_daemon::lifecycle: idle grace elapsed with no persistence reason - exiting idle_secs=32
2026-10-05T01:08:04.117053Z  INFO flowshot_daemon::daemon: daemon shutting down reason=IdleExit
```

Key timeline lines (stale-daemon self-check, proof2, T2 stderr retry chain, verbatim, ANSI-stripped):

```
DEBUG flowshot_cli::dispatch: forwarding to the running daemon member="Invoke"
DEBUG flowshot_cli::spawn: auto-spawned the helper daemon kind="direct"
```

### x11-phase-b/task6-launcher/ (12 files)

| File | Bytes | sha256 | PNG dims |
|---|---|---|---|
| RESULTS.md | 5769 | ead86ce235dcde05eda5a7bce508aaa6aee1b37afee5949974cbef78fcf8b6d1 | |
| l1-exploratory.log | 727 | f8a09a687d438432926664fd2348e36d85e3a3d0317b8d5cfbea9970ab20c6eb | |
| l2-cli-stderr-benign.log | 175 | 6f1872801818b265795104fd971a6d4085d781cfab2a659fe844ac42fa829a18 | |
| l2-submit.log | 520 | 8fdbfc1a8e4d7ba00e2d3bf69aa8846b36214bafeee7c03f2ecfa8ae1f24a16a | |
| l2b-submit-rerun.log | 449 | 837b16866fca5545f565af45498ba4b9ae8da6a6b60b9c8f209bfd0c63cf853a | |
| l3-cancel.log | 235 | 7d50eece4ab9488c29e25f359d12869f25e6c5d717b67716b87394871c225573 | |
| l4-popup-repro.log | 558 | cb1956c77b6556cef88cf4eddb5c29d775487d90343b6b7af9089e4f87f5c7f3 | |
| l5-popup-with-shots.log | 615 | 37652c47f4077130f3be42e0b045a58256af4ba49af9377cc56f126fb72942a8 | |
| launcher-dialog-opened.png | 15689 | 9c204c9bdcfcee0e1db6bf003cd2380832f745fdfb459ac347be76b3b3b1a0d8 | 900x396 |
| launcher-dropdown-edp1.png | 20938 | 23e391fe9fa7a51459c225a6537b786e5f44bdddf78229c55aec723e845cfbc9 | 900x700 |
| launcher-submit-400x300-saved-900x675.png | 241094 | 567b4652fc1e5422c092a2e713888615d94446b8be440e97adad640598185d1d | 900x675 |
| launcher-typed-geometry.png | 15454 | 1f621a97d32f0c2f96b68d7b663059ee62e5a979f590325676615d8289787c74 | 900x396 |

Totals: Phase A 21 files; Phase B 108 files (3 root + 2 task1 + 13 task2 + 24 task2-fresh + 34 task3-pins + 20 task4-riders + 12 task6-launcher). All sha256 sums in this manifest were recomputed against the on-disk artifacts at document generation time.

---

## 6. Regenerating / re-verifying

- Conventions (evidence classes, timeout-bounded + self-reversing live runs, namespaced artifacts, honesty rules): `docs/verification.md`.
- Phase A plan: `.omo/plans/x11-support.md` (task 8 = the Phase A evidence bundle).
- Phase B plan: `.omo/plans/x11-phase-b.md` (tasks 1-4 = live proofs, task 6 = consolidation + launcher gap-fill; task 7 owns docs).
- Re-verification on the source machine: rebuild (`cargo build`), re-run the literal commands quoted in each bundle's index/RESULTS under the same isolation pattern (private `DBUS_SESSION_BUS_ADDRESS` + private `XDG_CONFIG_HOME`, timeout-wrapped, XTest pre-flight via `xinput query-state 5/4`), then `sha256sum` the artifacts and compare against section 5. Identical bytes are expected only for deterministic artifacts (exit-code captures of static fixtures); live-screen captures will differ in the i3 bar clock region by design.
- This document itself: delete after handoff. It has no permanent-doc role; the durable records are the plan files, `docs/`, and the code.
