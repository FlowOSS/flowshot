# Issues — flowshot

Problems and gotchas encountered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## 2026-09-25: git repo never initialized (USER-CAUGHT DEFECT)
Todo 1 required `git init` (plan Commit strategy: "git init happens in todo 1") + one commit per todo. Worker claimed done without it; orchestrator verification missed it (ran build/test/clippy but never checked `git rev-parse`). Todos 1-4 landed uncommitted.
RECOVERY: git init -b main, reconstructed buildable sequential commits per todo (intermediate lib.rs states match each subagent's verified-green state: skeleton -> +tokens/config -> +geometry -> +scene).
PREVENTION: verification gate now includes `git rev-parse --git-dir` + `git status` after EVERY todo commit step; worker prompts explicitly forbid worker-side git (orchestrator owns commits, avoids index.lock races in parallel waves).

## 2026-09-25: wtype/ydotool absent + no passwordless sudo
Todo 1's live-input acceptance (wtype keystroke, ydotool mousemove vs hyprctl cursorpos) cannot run: tools not installed, `sudo -n` requires password. USER ACTION NEEDED: `sudo pacman -S wtype ydotool` (+ enable ydotoold for mouse). Until then: keyboard/mouse QA uses the todo-13 test-drive injection seam (plan-sanctioned fallback) and live runs avoid injection. grim+jq present; hyprctl IPC works (raw socket, no tools needed).

## 2026-09-25: downlevel wgpu limits = live-only bug class
Todo 13 passed every headless gate but panicked on the real 2-monitor session: downlevel limits cap textures at 2048px, DP-3 is 2560x1440. LESSON: headless-green != live-green for GPU code; with the GUI ban lifted, every GPU/window todo gets a timeout-bounded live run before commit. Plan needs 4K headroom (3840x2160 buffers, 4K frame budget) - device limits must never be downlevel defaults.

## 2026-09-25: core config gaps vs plan todo 2/28 spec (found in todo 28, core NOT editable from that task)
flowshot-core `SaveAction` only round-trips {copy, save, pin, upload}; the Amendment #3 `[save].actions` vocabulary also needs {copy-path, notify, open-with} (plan todo 2 + todo 28). `DaemonConfig` only has `tray` — plan todo 2 specifies `notifications=true` + `startup_launch=false` too. Todo 28 shipped the full vocabulary in `flowshot_actions::clipboard::Action` (+ `From<SaveAction>`) and takes `notifications_enabled: bool` as an executor parameter. ORCHESTRATOR ACTION: extend core config (or schedule a fix todo) before todo 32/35 wire config → executor; until then copy-path/notify/open-with are only constructible from CLI/daemon code, not from TOML.

## 2026-09-25: capture_icc BufferSizeMismatch on Rot90 headless output (FOREIGN FINDING, todo 7/9 territory — found during todo-15 live QA)
`hyprctl output create headless HS-1` + `hl.monitor({output="HS-1", transform=1})` (scale 2 default): probe reports the output correctly (physical 1920x1080, Rot90, logical 540x960), but `capture_icc --output HS-1` fails typed: "captured buffer (1920, 1080) does not match output buffer size (1080, 1920)". Hyprland 0.56.2 delivered the NATIVE-oriented buffer for the rotated headless output, while the todo-7 geometry guard expects the post-transform buffer_size (todo-7 learning "Hyprland sends m_transformedSize" was evidenced on transform-0 outputs only, where both coincide). Owner: capture-wayland (todo 9 worker is mid-refactor in icc.rs right now — do not patch from a UI todo). Backdrop side of the rotated+scale-2 chain is proven independently (todo-15 evidence item 7: 0.0000% vs grim -o oracle through the real render path). ALSO: `IccBackend::cursor_image()` returned null in the same session (cursor_pos --image prints null too) while cursor POSITION resolved fine via the ICC layer — suspect the in-flight todo-9 refactor; re-verify after todo 9 lands.

## 2026-09-25: todo 10 dep gate — RESOLVED (orchestrator added pipewire = "0.10.0" to root; re-dispatch landed COMPLETE, see evidence section 5)
Task-10 brief: "pipewire-rs and ashpd are workspace deps — verify versions in root Cargo.toml; if a dep is missing, STOP and report (root is read-only for you; orchestrator owns it)." VERIFIED: `ashpd = "0.10.0"` present (locked 0.10.3); **pipewire ABSENT** from [workspace.dependencies], Cargo.lock, and every crate manifest. Plan todo 1 (line ~120) confirms the design: "LATER-WAVE crates (pipewire-rs [todo 10], ...) enter the workspace table when their owning todo is reached" — the root-table add is the orchestrator's step and has not happened. Worker STOPPED per brief; zero code written, zero files touched outside evidence/notepad.
UNBLOCK (exact): add `pipewire = "0.10.0"` to root [workspace.dependencies] (Portal / D-Bus section), then re-dispatch todo 10. Readiness ALL verified (details in .omo/evidence/task-10-flowshot.txt + learnings.md): registry cache already holds pipewire/pipewire-sys/libspa/libspa-sys 0.10.0 (no network fetch); pkg-config libpipewire-0.3=1.6.8 + libspa-0.2 present (system-deps probe satisfied); clang/libclang 22.1 present (bindgen 0.72); pipewire 0.10 API matches plan flow (`Context::connect_fd(OwnedFd,..)`, `Stream::connect`, `dequeue_buffer`); ashpd 0.10.3 carries the full Screenshot+ScreenCast surface (source-verified); XDPH+portals+pipewire daemon running; pw-dump+grim present. MIT license = deny.toml-compatible; lockfile churn ≈ 4 new packages (bitflags/libc/rustix already locked). Re-dispatch note: `image` must move from flowshot-capture-wayland [dev-dependencies] to [dependencies] (zero lock churn) for the Screenshot decode path.

## 2026-09-26 (todo 20): [editor].mouse_preview config key ORPHAN
Plan todo-2 authoritative-naming rule lists `showMousePreview -> [editor].mouse_preview` as an ADDED key (Oracle r4 F-4 orphan fix), but the landed flowshot-core EditorConfig has no mouse_preview field. Todo 20 could not add it (core out of edit scope): `flowshot_ui::editor::EditorTools.mouse_preview` carries the spec default (true) as editor-side state. ACTION: whoever next owns a flowshot-core config edit adds `mouse_preview = true` to [editor] and wires EditorTools::from_config to read it (one-line projection change in flowshot-ui/src/editor/tool.rs).
## 2026-09-26 (todo 20): [shortcuts] config group still absent from core
Todo 2 spec mandates a [shortcuts] table (F12 defaults minus collisions + conflict-validation test); landed Config has no shortcuts group. ToolShortcuts (flowshot-ui editor/keys.rs) carries the F12 defaults + rebind seams; todo 25 (undo/redo config wiring) and todo 36 (settings UI) need the core group before persistence works.

## 2026-09-26 (todo 32): clipboard-offer RELEASE detection gap (for todos 35/38)
DaemonState::set_clipboard_offer_held is a MANUAL seam: wl-clipboard-rs 0.9.3 exposes no "selection replaced by another client" callback, so the daemon cannot observe when its data-control offer stops being the active selection. Set the flag when the daemon serves a capture copy; clearing needs either a data-control selection-event listener (wl-clipboard-rs gap) or an explicit release call from the copy pipeline. Until wired, an auto-spawned daemon that copied once persists until something else clears the flag (conservative direction: never kills a live offer).
## 2026-09-26 (todo 32): auto-launch / sd-notify crates absent from workspace table + lock
Plan todo 32 cites auto-launch for autostart; F21 lists it. Root Cargo.toml is orchestrator-owned and neither crate is locked, so todo 32 hand-rolled equivalents (zero-dep .desktop writer + NOTIFY_SOCKET datagram, both unit+live tested). ORCHESTRATOR: either accept the hand-rolled modules (recommended - they are injectable and validated) or add the crates to the root table and schedule a swap.
## 2026-09-26 (todo 32): plan acceptance "overlay appears" deferred to todo 35
Todo 32's plan acceptance line includes "overlay appears" after Invoke; per the task brief ("CLI wiring is todo 35") the daemon delivers the service shell + typed command seams (CommandSink/ChannelSink) and logs/queues commands. Todo 35 must plug the executing sink (capture dispatch) when wiring the CLI.

## Todo 35 BLOCKED (2026-09-26): clap_mangen missing from root workspace deps
- Task gate: "check workspace deps first; if clap_complete/clap_mangen missing from root, STOP and report" + MUST-NOT "NO root Cargo.toml/Cargo.lock".
- Verified: root `[workspace.dependencies]` has clap 4.5.16 (derive), clap_complete 4.5.16, clap_complete_nushell 4.5.16 — **clap_mangen ABSENT** (also 0 hits in Cargo.lock, not even transitive).
- This is a todo-1 execution gap: plan line ~120 (todo 1 dep-pin list) explicitly names `clap_mangen` alongside clap/clap_complete, but it never landed in the root table.
- Unblock = ONE line in root Cargo.toml `[workspace.dependencies]`: `clap_mangen = "0.3.3"` (crates.io current; MIT OR Apache-2.0 — deny.toml allowlist already covers; pulls clap_mangen + roff into the lock, small churn) + `clap_mangen = { workspace = true }` in crates/flowshot-cli/Cargo.toml. Must be done by whoever owns the root manifest (concurrent workers in flowshot-ui/flowshot-daemon make lockfile edits contention-prone — exactly what the STOP gate protects).
- Everything else for todo 35 is READY: flowshot-cli Cargo.toml already carries clap/clap_complete/clap_complete_nushell/tokio/anyhow/tracing + path deps (core/capture/ui/actions); daemon seam landed (request.rs `CAPTURE_OPTION_KEYS` 12-key vardict vocabulary + `CaptureRequest::from_vardict` parse boundary, bus.rs Invoke, flowshot-daemon helper binary); main.rs is still the stub.
- Man pages are not bolt-on: acceptance requires `man --warnings -l flowshot.1` render + flowshot-config.5, so the surface/generation design depends on clap_mangen being available.

## 2026-09-26 (todo 34): portal app-id launch requirements (todo 35/39 ACTION)
xdp 1.22.1 GlobalShortcuts requires a host app id: the daemon process must run under a systemd `app-org.flowoss.FlowShot-<RANDOM>.scope` (or app-...service) unit AND `org.flowoss.FlowShot.desktop` must be installed with a PATH-resolvable Exec (GLib rejects unresolvable Exec with a NULL GDesktopAppInfo — desktop-file-validate does NOT catch this). Without both, CreateSession fails NotAllowed "An app id is required" and FlowShot degrades to the compositor-bind fallback (which works, but loses portal dispatch). ACTION todo 35: CLI-spawned daemons should wrap the spawn in `systemd-run --user --scope --unit app-org.flowoss.FlowShot-$RANDOM` when systemd is present (init-agnostic: pure enhancement, fallback everywhere else). ACTION todo 39: the packaged .desktop (already planned) MUST be the real installed file; the systemd user unit `flowshot.service` does NOT match the app-* pattern — document that portal shortcuts need the scope-wrapped or desktop-launched daemon (autostart .desktop launches qualify via the DE's app-*-autostart units).
## 2026-09-26 (todo 34): Hyprland portal shortcuts need a `global`-dispatcher bind (todo 40 docs ACTION)
XDPH 1.4.1 drops preferred_trigger (registers with an empty trigger) and Hyprland only fires portal shortcuts through user binds: `bind = ,Print,global,org.flowoss.FlowShot:capture-region` (0.56 Lua: `hl.bind("Print", hl.dsp.global("APPID:NAME"))`). docs/setup-hyprland.md MUST document BOTH options: exec-CLI binds (no daemon residency) and global-dispatcher binds (portal dispatch, resident daemon). The plan's todo-34 QA wording ("portal-register Print -> wtype Print -> overlay") assumed automatic key grab — false on Hyprland (true on GNOME/KDE where the portal backend grabs keys; NOT live-verified here, no such sessions on the box).
## 2026-09-26 (todo 34): XDPH session-GC leak (FOREIGN, todo 38 QA hygiene)
Compositor global-shortcut entries survive the portal client's D-Bus disconnect (repro'd 2x, also after the daemon's graceful Session.Close): `hyprctl globalshortcuts` still lists them until `systemctl --user restart xdg-desktop-portal-hyprland`. Todo-38 flow (11) must include a post-run staleness check + XDPH restart cleanup; not a FlowShot defect (upstream XDPH/xdp session GC).
## 2026-09-26 (todo 34): plan QA mechanics stale on Hyprland 0.56 (todo 38 note)
`hyprctl keyword bind ...` (plan todo-34/38 fallback mechanic) is dead on the Lua config parser ("keyword can't work with non-legacy parsers. Use eval."); runtime binds = `hyprctl eval 'hl.bind(...)'` + `hl.unbind(...)`. wtype still absent system-wide (built from source in /tmp for QA; todo 38 needs a permanent answer — nix devShell per todo 39 lists it).

## 2026-09-26 (todo 33): core palette has no danger/attention token (todo 36 ACTION)
The tray attention-state glyph hardcodes red-600 (220,38,38) in flowshot-daemon/src/tray/icon.rs because flowshot-core's Palette carries only accent/contrast/dim_opacity. Todo 36 (settings UI, palette editor) should add a danger/attention token to core and switch icon.rs's ATTENTION_RGB to read it.

## Todo 35 BLOCKED entry RESOLVED (2026-09-26, re-dispatch landed COMPLETE)
clap_mangen = "0.3.3" was added to the root table by the orchestrator; the worker added it to flowshot-cli [dev-dependencies] (man generation = examples/man_pages.rs + tests, NOT the shipped binary; plan MUST-NOT "no hidden dev subcommands" honored). Lock churn = clap_mangen + roff only, as predicted. All readiness notes from the BLOCKED entry were used: CAPTURE_OPTION_KEYS vardict vocabulary (round-trip proptest through CaptureRequest::from_vardict), Invoke/Capture seam (typed-when-lossless mapping table in wire.rs), man acceptance (man --warnings -l clean on all 8 generated pages), todo-34 systemd-run scope wrap (LIVE-VERIFIED: app-org.flowoss.FlowShot-<nonce>.scope active with the auto-spawned helper inside). Evidence: .omo/evidence/task-35-flowshot.txt.

## 2026-09-26: kwin stub tests are load-sensitive (flaky under fresh-compile contention)
First full-workspace `cargo test` after a big compile batch failed `kwin::tests::stub_round_trip_area_capture_matches_the_raw_fixture` + `unknown_qimage_format_is_a_typed_decode_error` (30s runtime = deadline timeouts). Re-run after build cache warm: ALL green (988 passed). Root: the stub tests use deadline-bounded pipe reads; parallel rustc/test contention starves them. ACTION todo 38/CI: run the suite with warm cache, or bump the stub deadline budgets if CI shows this flake.

## 2026-09-26 (todo 26): core config gaps vs plan todo 26 (core NOT editable from UI tasks)
- `showSidePanelButton` (plan todo 26: "side panel (Space toggles, showSidePanelButton config)") is ABSENT from core UiConfig - no key to gate a toolbar panel-toggle button. Landed: Space toggle + [editor].side_panel gate. ACTION: core config owner adds [ui].show_side_panel_button; todo 36 wires the button.
- Core `default_toolbar_buttons()` (arrow,rectangle,circle,marker,text,pixelate,counter,copy,save,pin,upload,undo) does NOT match the plan's F12 default order (pencil,line,arrow,selection,rect,circle,marker,text,circlecount,pixelate,invert,move,undo,redo,copy,save,upload,open-app,pin,exit). The todo-26 contract (order = config list) works with ANY list; the DEFAULT is core's. ACTION: core config owner aligns the default (todo 36 validation).
- [editor].side_panel semantics: treated as the panel FEATURE gate (hard-off hides everything); initial visibility is hidden (Flameshot parity, Space opens). If the intended reading is "initial visibility", the toggle default flips in one line (ChromeState::panel_visible).

## 2026-09-26 (todo 26): deferred chrome polish (no plan acceptance line; needs new surface)
- Tooltips (400ms dwell) + toolbar reveal / panel slide animations (motion tokens D8d): need hover-dwell tracking (chrome has no motion route today) + a per-frame redraw scheduler (OverlayCore::tick only serves the selection HUD deadline). ACTION todo 38 (integration/polish wave): add a chrome motion/tick surface or drop the plan lines explicitly.
- SizeHud (chrome/hud.rs) is an inert seam: `visible` is never set in production and `rect` never positioned (the plan todo 26 text does not include the notifier box; todo 20 shipped the timing-based digit reset without a visual). ACTION todo 38: wire via a size-change EditorEffect + tick deadline, or delete.

## 2026-09-27 (todo 17): magnifier follow-ups for todo 35/36/38
- MAGNIFIER SAMPLES THE INSTALLED FRAME ONLY: cursor on an output whose frame is not installed (harness: first --frame output; production: todo-35 stitched-vs-per-output policy) -> magnifier hidden there. Same limitation as the todo-27 eyedropper; todo 35 owns the policy.
- NO NEUTRAL PALETTE TOKEN: magnifier grid ink (gray 128/96) + readout white text follow the paint_grid/size-HUD hardcoded precedents — todo 36 theming should add neutral/ink tokens and switch both (joins the tray danger-token action).
- app.rs at 246 pure LOC = WARNING BAND: the todo-35 binary-layer wiring must split before adding lines (render_window is the growth point).
- Grid toggle (F) is NOT re-projected by editor.configure() while the magnifier toggle (L) IS — inconsistent settings-apply semantics within EditorState; todo 36 (settings surface) should align grid_visible with the magnifier pattern.

## 2026-09-27: USER PRESENT — live GUI window QA re-suspended (policy change)
The user returned to the machine: visible windows/overlays are hostile again. Effective
immediately and until the user explicitly re-allows: NO visible-window QA runs (overlay
harnesses, pin windows, dialogs). Verification falls back to: `--verify-offscreen` headless
render paths (frozen_backdrop/pin harnesses), unit + integration seams, and invisible
protocol reads. Anything genuinely needing a visible window (fullscreen placement asserts,
hyprctl clients checks, focus behavior) is queued into `.omo/evidence/gui-qa-batch.md` for a
user-approved batch run later. If a completed todo's live QA was interrupted by a user Esc,
that is NOT a defect — re-verify offscreen instead of re-running visible.

## 2026-09-27: CPU policy — half cores while user is present
User directive: builds cap at 8/16 jobs while they use the machine (full CPU fine when away).
Enforced via .cargo/config.toml [build] jobs = 8 (auto-applies to every worker's cargo calls —
no prompt compliance needed). REMOVE or raise when the user steps away. Side benefit: the
load-sensitive kwin stub tests (deadline budgets) should flake less under a capped build.

## 2026-09-27 (todo 18): follow-ups for todo 38/40 (+ one DEFERRED GUI item)
- TODO 38 (binary layer): consume the launch.rs module-header MAPPING TABLE — build LaunchRequest from the CLI's RegionToken/CaptureRequest + config ([capture].save_last_region, [capture].last_region) + todo-12 resolve_cursor_pos; call core_mut().launch(request) BEFORE run() (production order: the spawn hook resolves it on Resumed) and set_region_sink(load-or-default -> set -> save) — frozen_backdrop's apply_launch_args() is the reference wiring. `capture screen` no-arg: output_at_cursor(layout, position) -> Option<&OutputInfo>; None = CALLER's fallback policy (undecided: first output vs typed error — no overlay exists to await motion on for a non-interactive capture). Toolbar copy/save button wiring (W5 callbacks) must route capture-completing effects through the funnel's main exit or call launch_persist explicitly — the chrome-consumed early-return branch deliberately skips region persistence.
- TODO 38/40 (confirm): `--region at-cursor` shipped as Preselect::OutputAtCursor (whole output under the cursor; recorded decision in decisions.md — the CLI's RegionToken::AtCursor doc is ambiguous and no spec defines a size-less centering). Docs (todo 40) must state the chosen semantics; a remap is a one-variant change in launch/resolve.rs.
- DEFERRED (gui-qa-batch.md, todo-38 gate + visible-window ban): end-to-end `flowshot capture --region 200x100+50+50 --instant` + click + `-o` save == 200x100 grim-oracle crop, and `capture screen` no-arg saved-PNG dimensions. The ui-side halves are closed invisibly (seeded geometry tokens + pixel asserts + live output-at-cursor == hyprctl oracle + centered == live cursorpos reading).

## 2026-09-27 (later): USER AWAY (sleeping) — GUI QA re-allowed + full CPU
User went to sleep: visible-window QA, input injection, and compositor state changes are
ALLOWED again (standing rules: timeout-bounded, self-reversing, no leftover state). The
.cargo/config.toml jobs=8 cap is REMOVED (full 16 cores for builds). The deferred items in
.omo/evidence/gui-qa-batch.md may now be executed and closed out.

## 2026-09-27 (todo 36): VENDORED INTER FONTS WERE CORRUPT (foreign todo-19 defect — FIXED in place)
crates/flowshot-ui/fonts/Inter-{Regular,Medium,SemiBold}.ttf were GitHub 404 HTML pages (~268KB each, `<title>Page not found · GitHub</title>`) — the todo-19 vendor step saved error pages instead of fonts. Latent until todo 36: NOTHING consumed them before (the GPU text stack uses cosmic-text FontSystem::new() = SYSTEM fonts via fontdb; build.rs only rasterizes icons). The settings surface's include_bytes → epaint install failed ("Error parsing Inter TTF/OTF: InvalidFont"). FIXED: replaced all three with genuine Inter 4.1 TTFs from the rsms/inter GitHub release (OFL-1.1; fonts/LICENSE.txt was already the real OFL text), `file`-verified TrueType. ACTION todo 41 (visual QA): the editor's [editor].font_family default "Noto Sans" resolves via fontconfig (unaffected), but any future include_bytes consumer of fonts/ now gets working files — re-verify Inter rendering at scale factors during the visual pass.

## 2026-09-27 (todo 36): root-table follow-ups for the orchestrator (NON-blocking)
- egui-winit = "0.28.1" in the root [workspace.dependencies] is a DEAD pin: it requires winit ^0.29 (workspace = 0.30.10) and no egui release pairs (wgpu 0.20, winit 0.30) — verified against the cached registry. Unused by any crate; safe to drop from the root table (or keep as a documented trap). The plan's Metis-#5 note ("egui 0.36.2 -> winit ^0.30.13 OK") is stale for THIS workspace: egui 0.29+ requires wgpu 22+, and wgpu must not move (todo 13 blast radius).
- egui inherits default features (workspace inheritance FORBIDS default-features=false overrides — cargo error verified) → egui's embedded default_fonts (~1MB) ride along UNUSED (the vendored Inter replaces them at runtime). Optional trim: set `egui = { version = "0.28.1", default-features = false }` in the ROOT table (flowshot-ui installs Inter unconditionally, so nothing breaks; verify no other future consumer wants the defaults).

## 2026-09-27 (todo 36): binary-layer wiring checklist for todo 38 (settings window)
SettingsWindow is code-complete but NOTHING spawns it yet: daemon DaemonCommand::Settings still lands on LoggingSink/ChannelSink. Todo 38 must: (1) spawn SettingsWindow from the Settings command (daemon side: process or thread — the window blocks its event loop) with options.config_path = flowshot_daemon::paths::default_config_path(); (2) WindowCustomizer → app_id=flowshot-settings (Wayland ext, pins precedent); (3) AppliedCallback → emit ConfigChanged on the bus (NOTE: the signal does NOT exist yet on the org.flowoss.FlowShot interface — todo 32 shipped methods only; either add the signal or re-read config on next capture) + re-project live surfaces (EditorState::configure, chrome toolbar re-read, daemon tray/notifications toggles); (4) PathPicker → rfd; (5) ClipboardBridge → wl-clipboard; (6) ashpd Settings portal → options.system_theme + handle.set_system_theme on change. Deferred live-QA items: gui-qa-batch.md "Todo 36" section.

## 2026-09-27 (todo 36): core-config gaps RE-CONFIRMED (still orchestrator-owned; settings edits what EXISTS)
The todo-26/33/17 core actions remain open and now block settings-surface completeness: [ui].show_side_panel_button absent (no key to gate a panel-toggle toolbar button); core default_toolbar_buttons() ≠ plan F12 default order (settings edits whatever list core ships — the Interface tab's add-vocabulary already covers the full F12 id set); Palette has no danger/attention + neutral/ink tokens (tray icon.rs ATTENTION_RGB + magnifier grid ink stay hardcoded). One NEW gap: [shortcuts] group absence now directly blocks shortcut PERSISTENCE (recorder works, Apply cannot write bindings — model state only). All four = one core-config edit todo.

## 2026-09-27 (todo 37): follow-ups for todo 38/40/41 + orchestrator
- TODO 38 (binary layer, launcher): spawn LauncherWindow from DaemonCommand::Launcher (the SettingsWindow checklist entry's twin; the window blocks its event loop - process or dedicated thread). Wiring reference = examples/launcher_dialog.rs: MonitorProbe -> CaptureThread::outputs (the tray's spawn_blocking-per-probe pattern works), LaunchCallback -> Region => Capture(CaptureRequest{region: Some(geometry.to_token()), delay_ms, ..}) / Screen => CaptureScreen(screen), WindowCustomizer -> with_name("flowshot-launcher", "Capture Launcher"), CLI one-shot --dialog path -> Cancel = exit 3 (user-cancelled). The CaptureScreen delay_ms mapping is OPEN (wire slot has no delay): honor via Capture(region=monitor logical rect)+delay, extend the frozen wire (avoid), or document-drop the delay for Screen - todo 38 decides.
- TODO 38/40 (grammar unification): the WxH[±X±Y] parser now exists TWICE (flowshot-cli invocation.rs parse_region_token + flowshot-ui launcher/request.rs RegionGeometry::parse - rules mirrored 1:1, roundtrip-tested). Shared home = flowshot-core (orchestrator-owned from UI tasks); a core edit todo should host one parser + one typed rect and re-point both consumers.
- TODO 41 (visual pass): the launcher dialog is 400x232 with ~90px bottom whitespace in the normal state (error-label headroom kept); tighten or bottom-anchor the button bar. The egui error_fg_color is the egui default (theme.rs doesn't project a danger token - joins the tray ATTENTION_RGB + magnifier-ink token actions). Launcher screenshot set for the visual bundle: .omo/evidence/task-37-flowshot.png row 1.
- ORCHESTRATOR (non-blocking): settings::SettingsSurface was RETIRED in the egui_host extraction (EguiSurface is crate-private; settings' public API otherwise unchanged - render_offscreen/OFFSCREEN_FORMAT/ClipboardBridge/ThemeMode/style/fonts/parse_hex_rgb all still exported from flowshot_ui::settings). No consumer existed outside the crate (todo 38 wires SettingsWindow, not the surface). Root-table egui-winit dead pin note (todo 36) unchanged.

## 2026-09-27 (reboot): user PRESENT again — policies re-applied
Machine hung and was rebooted during todo 38's run (unknown cause; todo 38 was doing
live fullscreen overlay QA at the time — record, don't assume). Policies back to
user-present: NO visible windows (offscreen QA only), jobs=8 cap re-applied via
.cargo/config.toml. Todo 38's worker left: completion.rs (new) + 4-crate edits, build
broken with 4 errors at reboot time.

## 2026-09-27 (todo 38): winit 0.30.13 = ONE EventLoop per PROCESS (architecture-forcing)
`EventLoopBuilder::build()` flips a process-global `EVENT_LOOP_CREATED`
AtomicBool and returns `Err(RecreationAttempt)` on ANY second build —
regardless of thread or `with_any_thread` (registry-source-verified). The
daemon-thread window-hosting model (todo 30/32/36/37 notes "process or
dedicated thread") is therefore IMPOSSIBLE in-process: todo 38 landed the
child-process model — every window session (overlay/color/pin/launcher/
settings) runs as `<current_exe> session --spec <json>` (hidden verb on
BOTH binaries; internal process-model contract, not a dev subcommand).
The PARENT runs post-capture so the clipboard offer stays daemon-owned
(todo 28) and the pin registry/persistence reasons stay in one process.
Consequence (documented trade-off): a PIN's own copy/save offers are
served by the pin child and die when the pin closes — a pin-copy
outliving its pin needs a bus-level clipboard handoff (frozen todo-32
vocabulary has no member; roadmap).
## 2026-09-27 (todo 38): xdp app-id needs a DECIMAL scope nonce + installed .desktop
Live flow-11 probe: `app-org.flowoss.FlowShot-t38-<pid>.scope` FAILED
registration ("An app id is required") — xdp derives the app id by
splitting the unit at the LAST dash, so an extra dash segment corrupts it
to `org.flowoss.FlowShot-t38`. `app-org.flowoss.FlowShot-<pid>` (decimal)
+ a NoDisplay `org.flowoss.FlowShot.desktop` fixture => registration
SUCCEEDED live (compositor listed all 3 shortcuts). FIXED: cli spawn.rs
`scope_unit` hex -> decimal (test updated). ACTION todo 39: the packaged
.desktop MUST be installed for portal shortcuts (known since todo 34, now
live-proven both halves).
## 2026-09-27 (todo 38): ICC rotated-headless finding CONFIRMED still present (foreign)
Flow 13 live probe: Hyprland 0.56.2 still delivers the NATIVE-oriented
buffer for a Rot90 headless output through ext-image-copy-capture (the
todo-7/9 `BufferSizeMismatch` class). NEW: the executor's runtime ladder
fallthrough (probe-green != capture-green) absorbs it — wlr-screencopy
served the rotated scale-2 output correctly (1080x1920 upright physical).
No FlowShot-side fix possible; keep the fallthrough + per-rung warn logs.
## 2026-09-27 (todo 38): perf 1080p budget DEGRADED on this NVIDIA box
wgpu bring-up (Instance 36-47ms + adapter/device ~82ms = 118-139ms) alone
consumes 79-93% of the 150ms 1080p budget; composed estimate ~180-210ms
(07-perf-budgets.md). Dual budget PASSES (290-348ms worst-case offscreen
proxy <= 400ms). Remediation design recorded: prewarm the wgpu Instance
concurrently with the capture, then full GPU||capture overlap in the
session child (-> ~145ms). ACTION todo 41: implement + measure the TRUE
hotkey->present live.
## 2026-09-27 (todo 38): follow-ups for todos 39/40/41/42
- TODO 39: ship the real `org.flowoss.FlowShot.desktop` (portal shortcuts
  need it installed); nix devShell must carry wtype+ydotool (absent since
  the reboot — flow 11's live trigger is gated on it); rfd (or a portal
  PathPicker alternative) for the settings Browse button (stays None).
- TODO 40: docs must state: `--region at-cursor` = whole output under
  cursor (todo-18 decision, confirmed); `capture screen` no-arg with an
  UNRESOLVED cursor falls back to the FIRST output + warn (todo-18 open
  question decided in todo 38); `--raw` is EXCLUSIVE stdout mode,
  `--print-geometry` is ADDITIVE (prints then runs actions); one-shot
  `--no-daemon` clipboard offers die with the process (help text says so);
  pin-child offer lifetime (above); editor sampling frame = stitched
  scale-1 (magnifier/pixelate/eyedropper sample at LOGICAL resolution on
  scale>1 outputs — exact at scale 1; per-output alternative was blind
  beyond output 0).
- TODO 41: live perf re-measure + the gui-qa-batch todo-38 section;
  overlay app_id=flowshot assert (the with_window_customizer seam landed;
  todo-13 deviation A closes there).
- TODO 42: cross-platform note — the child-process session model is also
  the macOS/X11 answer (winit main-thread requirements); the
  `EventLoopHook`-style platform seams stayed OUT of flowshot-ui (purity
  held: no winit platform imports in ui).
- Clipboard release detection (todo-32 gap) unchanged: the executor SETS
  clipboard_offer_held on copy; clearing still needs a data-control
  selection listener (wl-clipboard-rs gap).
- Settings ConfigChanged bus signal still absent (frozen vocabulary):
  Apply = per-execution config re-read + log projection (todo-36 decision
  executed).

## 2026-09-27 (live-found defect): interactive capture/settings DEAD - nested-runtime panic in the CLI session child + daemon silent failure (FIXED)
USER-FOUND DEFECT CLASS (feeds F3): every todo-38 headless-verified flow passed while the REAL window path was dead on arrival. `flowshot capture --no-daemon` panicked ("Cannot start a runtime from within a runtime", overlay/mod.rs:134); `flowshot capture` via daemon failed SILENTLY (CLI exit 0, no output - the daemon's child died and the failure only hit the daemon log). The headless execution mode never exercises the session-child verb, so the GUI batch had zero coverage of the one path users actually run.
ROOT CAUSE 1: flowshot-cli main.rs dispatched the hidden `session` verb INSIDE its tokio runtime (dispatch.rs arm) - overlay_child/settings_child/launcher legs build their own runtimes, so the nested block_on panicked. The flowshot-daemon binary already early-dispatched `session` in main() before its runtime; the CLI did not.
FIX 1 (unrepresentable, not patched): `Session` removed from `Invocation`; resolve() returns `Resolved::{Session, Command}`; main runs run_child on the MAIN thread pre-runtime; dispatch's async surface can no longer name the verb. All four SessionKinds fixed at once.
ROOT CAUSE 2: bus methods were sync fire-and-forget - the Ok reply was sent before execution started; ExecutingSink errors went to tracing only.
FIX 2: CommandSink::dispatch_tracked -> ExecutionReceipt (oneshot); ExecutingSink resolves it with the execution result bounded by STARTUP_REPLY_WINDOW=5s (still-running at window = accepted Ok, preserving fire-and-forget UX for open sessions/delays); bus methods async-await the receipt: Err -> fdo::Error::Failed(message), dead executor thread -> Failed(EXECUTOR_THREAD_DIED). CLI's existing CliError::Dbus mapping gives exit 1 + "flowshot: D-Bus failure: ...: session child failed ...". Wire vocabulary untouched. LIMITATION (deliberate, documented in sink.rs): failures AFTER the 5s window stay daemon-log-only.
COLLATERAL FINDING (the new e2e test exposed it): Heartbeat::drop joined a worker parked in a NON-interruptible 20s sleep - every session result was delayed up to 20s (stalling the executor's single-thread runtime; would have defeated the reply window). Heartbeat now parks on mpsc::recv_timeout + prompt stop signal. e2e chain: 20.09s -> 0.27s.
TESTS: session_dispatch.rs (real binary + real session verb, headless: graceful typed Failed result, no panic, exit propagates the mapped code); daemon_failure_surface.rs (REAL daemon on a private dbus-daemon bus + production ExecutingSink + REAL flowshot binary: failing child -> CLI exit != 0 + "session child failed" on stderr); 3 bus receipt-mapping tests; parse_matrix session-lane test. Gates green: 1168 tests, clippy -D warnings, fmt.
LIVE (bounded, self-reversing, QA config seeded/restored): one-shot overlay PRESENT + clean cancel (exit 3 = the frozen table's user-cancelled code - the task brief's "Esc exits 0" disagrees with exit.rs; 3 is correct and documented); daemon-path overlay PRESENT (2 windows/2 outputs) + CLI exit 0 + clean close, daemon log zero failures; settings window PRESENT + clean close; daemon idle-exited by itself; `capture full --no-daemon --raw` still exit 0 with a valid 4480x1440 PNG (one-shot direct path untouched). Evidence: .omo/evidence/fix-overlay-session-panic.txt + fix-*-live*.png screenshots.
QA-TOOLING FINDING (gui-qa batch + todo 41): Hyprland 0.56.2 REMOVED the legacy `hyprctl dispatch killactive/focuswindow` string syntax - runtime window control needs the lua dispatchers now: `hyprctl dispatch 'hl.dsp.window.close({window = "class:flowshot$"})'` (selector-targeted: can never hit a user window), `hl.dsp.focus({window = "class:..."})`. The flow scripts' killactive usage is dead on this compositor.

## 2026-09-27 (STRICT): user is ACTIVELY USING the machine (gaming) — zero visible windows, no exceptions
The user is playing a game; ANY window/popup from FlowShot QA is hostile. NO visible-window
runs of any kind, including "one brief fix-proof" runs. Verification = offscreen renders +
unit seams + invisible protocol reads ONLY. The USER verifies visible behavior when they
choose to test. jobs=8 cap stays. (Orchestrator note: pkill -f "flowshot" from a shell
command containing that string kills the command itself — use pgrep/pkill with exact names.)

## 2026-09-28 (todo 42): STOP-and-report findings for the orchestrator / F-wave
- F-1 LICENSE IDENTITY MISMATCH (F4-relevant, owner decision): ALL 7 crate manifests declare `license = "MIT OR Apache-2.0"` (crates/*/Cargo.toml:5) vs root [workspace.package] `license = "GPL-3.0-or-later"` + README + the announced user-approved default. Crates don't use `license.workspace = true`. Fix = 7 one-line edits; NOT done (license identity = owner call). NOTICES states GPL-3.0-or-later per README/workspace.
- F-2 NO ROOT LICENSE FILE (ls -la verified). F4 expects "license file GPL-3.0-or-later". Add with packaging (todo 39) or F-wave.
- F-3 NO justfile at root (plan todos 1/39 + the `just check` commit gate reference it).
- F-4 ROOT MANIFEST DRIFT vs PLAN PINS (root Cargo.toml = STOP-and-report scope): zbus = "4.3.1" (plan: zbus 5; the ONLY root-actionable cargo-tree-d d group - zbus 5 already in lock via ashpd 0.10.3 + notify-rust 4.18; bumping collapses zbus/zvariant/zbus_macros/zbus_names/zvariant_derive/zvariant_utils pairs); ashpd = "0.10.0" (plan: 0.13); egui-winit = "0.28.1" is a DEAD workspace-table entry (deliberately unused - ui Cargo.toml documents the winit 0.29-vs-0.30 reason).
- F-5 CI `cargo deny check` was RED since todo 1: deny.toml carried non-SPDX "Unicode" -> cargo-deny 0.20 refused to PARSE the config. Fixed within the 42(d) finalize mandate (Unicode-3.0 + OFL-1.1 + 4 forced additions + epaint UFL exception + 3 per-ID RUSTSEC ignores + bans warn). Green now: advisories ok, bans ok, licenses ok, sources ok.
- F-6 (informational, F2) ci.yml clippy/test steps lack --workspace; harmless in a virtual workspace (default-members = all members).
- TODO 42 delivered: scripts/purity-gate.sh (+allowlist, 2 recorded ui dev-dep entries) wired into ci.yml, PASS clean / FAIL planted (3/3 pattern classes) / revert byte-identical; unsafe audit (zero unsafe blocks workspace-wide; capture-wayland = the single recorded exemption, unused); NOTICES (643 crates, 27 license groups, OFL/ISC/MIT full texts, exact 11-icon Feather subset); docs/porting-roadmap.md (4 phases, >=3 spot-checked crates each, F20 conformance table, F24 red flags) + ADR-006 addendum. Evidence: .omo/evidence/task-42-flowshot.txt.
- flowshot-ui purity: CLEAN (zero violations) - concurrent todo-41 worker unaffected; ui untouched by todo 42.

## 2026-09-28 (sleep): user AWAY again — full QA + full CPU; gate ceremony reduced
User asleep: visible-window QA re-allowed (timeout-bounded, self-reversing). jobs=8 cap
REMOVED. User directive: skip the per-completion full-build gate ceremony — workers run
their own gates; orchestrator does targeted checks + ONE consolidated full verification
at the end (avoids redundant workspace rebuilds between waves).

## 2026-09-28 (todo 41): follow-ups recorded
- REDUCED-MOTION CONFIG KEY: the UI switch is complete (OverlayCore::set_motion_reduced,
  PinBehavior.reduced_motion) but `[ui].reduce_motion` needs a flowshot-core config key +
  settings-Interface toggle + binary-layer wiring (daemon executor/session specs) — outside the
  todo-41 edit scope (flowshot-ui only). Small: one bool key, one projection call per session
  child (overlay/pins), one settings row.
- WGPU PREWARM REMEDIATION (todo-38 perf follow-up "ACTION todo 41"): NOT implemented — lives in
  flowshot-daemon::execute (Instance prewarm concurrent with capture), outside this dispatch's
  edit scope; live hotkey→present re-measure needs a visible window (gui-qa-batch todo-41 row 3).
  Re-dispatch both together.
- LIVE IDLE-CPU + motion-feel + app_id asserts: gui-qa-batch.md todo-41 rows 1/2/4/5 (zero-
  visible-windows policy; unit frame-schedule asserts + offscreen bundle landed as substitutes).
- side_panel/paint.rs sits at 230 pure LOC (warning band) — next edit there should split the
  layers-section half.

## 2026-09-28: evidence-file race — orchestrator instruction deleted a real artifact
The tail worker was instructed to "delete the stray 0-byte task-14-flowshot.png" WHILE the
orchestrator was concurrently re-capturing the real oracle at the same path — the worker
deleted the real 7.3MB capture. LESSON: never hand a worker a destructive instruction on a
path the orchestrator is actively writing; sequence destructive steps or namespace them.
Caught by the F1 re-reviewer's filesystem check (the txt claimed the artifact existed).

## 2026-09-29: portal_ladder_end_to_end joins the load-sensitive test class
crates/flowshot-daemon/tests/portal_shortcuts.rs::portal_ladder_end_to_end failed once under
full-workspace load (15s runtime = its internal timeout tripped); passes 3/3 isolated (0.37s).
Same class as the kwin stub deadline tests. ACTION (CI/todo): the private-broker stub tests
should use generous-but-bounded waits; if CI shows this flake, bump the stub timeouts.

FIXED: Increased registration_timeout from 15s to 30s and wait_for timeout from 15s to 30s
in portal_shortcuts.rs to prevent load flaking.

## 2026-09-26: kwin stub tests are load-sensitive (flaky under fresh-compile contention)
First full-workspace `cargo test` after a big compile batch failed `kwin::tests::stub_round_trip_area_capture_matches_the_raw_fixture` + `unknown_qimage_format_is_a_typed_decode_error` (30s runtime = deadline timeouts). Re-run after build cache warm: ALL green (988 passed). Root: the stub tests use deadline-bounded pipe reads; parallel rustc/test contention starves them. ACTION todo 38/CI: run the suite with warm cache, or bump the stub deadline budgets if CI shows this flake.

FIXED: Increased wait_until timeout from 200ms to 500ms in kwin/tests.rs to prevent load flaking.

## Wheel sizing "broken" on shape tools (fixed 2026-10-02)
- **Report**: wheel tool-size works for counter/text, not for shapes (line/arrow/rect/ellipse/pencil/marker).
- **Measured (injection-seam probe, per-tool table)**: the wheel MECHANISM was never broken - accumulator -> apply_size -> on_size_changed -> committed object -> Redraw all fire for every tool. What was broken was FEEDBACK: (1) `ChromeState::show_size_hud` was dead code in production (only chrome/tests.rs called it) and `SizeHud.rect` was never assigned (zero rect = invisible even when visible=true; no hide timer either); (2) RectTool's hover dot painted from the persisted config `draw_thickness`, so wheel-on-rect (corner-radius slot) changed NOTHING visibly until a drag. Counter/text only "worked" because their previews change dramatically (bubble 16->24px, edit-widget font); shape dots move ±0.5px - imperceptible.
- **Fix**: `EditorUpdate.resized` flag (digits + wheel fallback set it) -> funnel flashes the chrome SizeHud; SizeHud reworked to a single `Option<Instant>` deadline (visibility IS the deadline), 600ms auto-hide via `OverlayCore::tick`/`wake` (Flameshot notifierbox parity); rect hover dot now follows the dispatched size (Flameshot `paintMousePreview` parity).
- **Evidence**: `.omo/evidence/wheel-sizing/` (assertion table 8/8 PASS, before/notifier/expired frames per tool, HUD crops, region diffs). Harness: `cargo run -p flowshot-ui --example wheel_sizing --features test-drive -- OUT_DIR`.

## 2026-10-03 (telemetry-core): follow-ups for the orchestrator / ui dialog task
- README.md is STALE in two spots (root file - "NO other root edits" constraint honored, orchestrator-owned): the config sample still says `config_version = 2` (fresh files now write 3 + the [telemetry] group), and the "What FlowShot deliberately does not do" list still claims "no telemetry" (now: opt-in, off by default, asked once). docs/config-reference.md + flowshot-config(5) roff ARE updated.
- UI FIRST-LAUNCH DIALOG TASK contract (stable, shipped): `flowshot_daemon::telemetry::{init, capture_error, capture_message, capture_error_tagged, capture_error-tagged aside, is_enabled, note_gpu_adapter, init_with_transport, Surface}` + core `[telemetry]` fields `enabled` / `include_technical_details` / `asked_on_first_launch` (all default false). The dialog writes the answers via Config::save (migration-safe) and flips asked_on_first_launch; capture_message takes `sentry::Level`.
- `note_gpu_adapter` pass-through is UNWIRED: flowshot-ui's gpu.rs selects the adapter but only LOGS its name (no public accessor). Whoever exposes it (ui task) should call `flowshot_daemon::telemetry::note_gpu_adapter(&info.name)` from the session-child legs after GPU init; until then tier-2 gpu_adapter = the /sys PCI-id fallback ("<driver> <vendor>:<device>").
- WARNING BAND (200-250 lib-pure LOC, split before adding lines): telemetry/environment.rs 210, execute/session/mod.rs 214 (pre-existing 212). PRE-EXISTING OVER: flowshot-core config.rs 351 lib-pure (pure serde schema/data-table exception; the task mandated the [telemetry] group IN config.rs - splitting into a config/ module dir is an orchestrator-scope refactor).
- Tier-2 monitor probe connects to the compositor at INIT (CaptureThread, tray pattern): with include_technical_details ON, every CLI invocation pays one extra bounded wayland probe. Documented trade-off (tier 2 is opt-in, default OFF); if it ever matters, cache the layout daemon-side.
- A daemon-forwarded execution failure produces TWO events by design (daemon process: typed ExecuteError with backend tag; CLI process: CliError::Dbus at the report() boundary) - different surfaces, one per failing process. If event volume matters, the CLI-side capture could skip the Dbus class.

## 2026-10-03 (telemetry-ui): follow-ups for the orchestrator / daemon wiring
- THE CONSENT DIALOG IS SHIPPED BUT NEVER SPAWNED: `flowshot_ui::consent` (ConsentWindow/ConsentWindowOptions/ConsentCallback/should_prompt) is complete + tested + QA'd, but no binary layer calls it yet. Daemon task: at startup, gate on `consent::should_prompt(&config.telemetry, PromptSurface::DaemonStartup)` and spawn the dialog as a DETACHED session child (needs a Consent SessionKind + wire vocabulary); the child leg wires tokens/ui_config/system_theme, `app_id=flowshot-consent` (WindowCustomizer), and ConsentCallback -> read-modify-write `config.telemetry` + `Config::save`. Never await the child (no command may block on consent).
- `note_gpu_adapter` pass-through is STILL unwired (prior entry unchanged) - the consent task did not touch gpu.rs; the adapter-name accessor + the session-child call remain open.
- README staleness (prior entry) now also covers the consent dialog + settings Telemetry card ("no telemetry" claim, settings-UI feature list).
- The consent window has NO test-drive injector (unlike launcher/pins): offscreen renders (both checkbox states), headless egui::Context widget tests, and a live visible run (evidence consent-live.png, grim-verified) cover the surface. Add a LauncherInput-style seam only if synthetic click QA becomes necessary.

## 2026-10-03 (gpu-warnings-cleanup): follow-ups for the orchestrator
- flowshot-daemon/src/execute/headless.rs still creates its wgpu instance with the IMPLICIT `..InstanceDescriptor::default()` (task rule limited this pass to flowshot-ui). Behavior is identical to the new explicit policy (debug=VALIDATION|DEBUG, release=empty), so the debug-log validation warning can still originate from the daemon's headless-capture instance; a daemon-side switch to `flowshot_ui::gpu::new_instance()` (or a daemon-local explicit form) is a one-line follow-up.
- gpu/surface.rs is at 217 pure LOC (warning band 200-250): split before the next line-adding edit (250-LOC ceiling).
- The user-log noise classes deliberately left untouched (task out-of-scope): zbus property-cache warnings (ashpd teardown race, benign), NVIDIA "Unrecognized present mode 1000361000" spam (driver-side, wgpu-hal conv).

## 2026-10-03: wgpu 0.20 swapchain semaphore reuse trips VUID-vkQueueSubmit-pSignalSemaphores-00067
With vulkan-validation-layers installed + a debug build (validation on by design), wgpu 0.20's
Vulkan backend emits ERROR-level validation complaints about present-semaphore reuse
("Swapchain image N was presented but was not re-acquired..."). UPSTREAM, not FlowShot: the
pattern is wgpu 0.20's swapchain semaphore pool; fixed in later wgpu (per-image semaphores /
swapchain_maintenance1 fences). Benign on NVIDIA in practice (renders correct, no corruption).
UPGRADE BLOCKER: egui-wgpu 0.28.1 pairs with wgpu ^0.20 — the wgpu bump waits for an egui
release that pairs with a fixed wgpu (revisit when egui ships a wgpu-22+ release and our
winit 0.30 constraint allows). Do NOT log-filter these: they are what validation is for.
