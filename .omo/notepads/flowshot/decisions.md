# Decisions — flowshot

Architectural choices and rationales discovered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## 2026-09-26 (todo 32): daemon executor, naming, and dependency decisions
- ZBUS EXECUTOR (task MUST-DO #1): daemon's own bus = workspace zbus 4.4 (async-io reactor) driven from a multi-thread TOKIO runtime; zbus 5 direct dep REJECTED (ashpd/notify-rust carry it internally; per-connection private executor + dedicated driver thread verified in zbus source - no conflict). Explicit Connection::close() on every teardown path (no drop-time close in zbus-4-async-io).
- BIN NAME: flowshot-daemon crate bin renamed `flowshot` -> `flowshot-daemon` (collision with flowshot-cli's `flowshot`; todo 35's `flowshot daemon` subcommand calls flowshot_daemon::{Daemon, DaemonOptions}).
- AUTOSTART: hand-rolled .desktop writer instead of the plan-cited auto-launch crate (absent from workspace table + lock; root manifests orchestrator-owned). Same file format, injectable dir, zero deps. Orchestrator may swap in auto-launch later if the root table grows it.
- SD_NOTIFY: hand-rolled NOTIFY_SOCKET datagram behind NON-default `systemd` cargo feature (sd-notify crate absent from lock). Init-agnostic per Amendment #3.
- IDLE GRACE: DEFAULT_IDLE_GRACE = 60 s (pub const; plan fixes no constant; --idle-grace flag overrides).
- CAPTURE WIRE FORMAT: Capture(options) = a{sv} vardict (portal/KWin idiom) parsed once into typed CaptureRequest; unknown keys warn+ignore (forward compat), wrong types = InvalidArgs reply. Wire keys = Amendment-#2 modifier names.
- DISPATCH SEAM: bus methods -> DaemonCommand -> CommandSink (LoggingSink default; ChannelSink for todo 35's executor consumer; RecordingSink for tests). Capture execution/overlay launch = todo 35/38 (task brief: "CLI wiring is todo 35").
- FOREIGN-NAME CLASSIFICATION: no-reply-until-timeout folds into ForeignNameHolder (a zbus peer without an object server silently drops calls - source-verified); OwnerVanished (NameHasNoOwner) stays separate (remedy = retry).

## 2026-09-26 (todo 34): global-shortcuts design decisions
- PORTAL CLIENT = ashpd 0.10.3 (the brief's dep) over a zbus-4 reimplementation: ashpd's wire behavior (path conventions, 's'-or-'o' session_handle quirk, Request/Response joining) is battle-tested against real portals, and the stub-portal e2e exercises the REAL production path (env-steered OnceLock) — so "never claim unexercised portal paths" holds without duplicating the protocol. Recorded pins: no restore tokens, pub(crate) session handle, internal assert (spawn-contained), process-global session connection.
- RESTORE DATA = re-registration set + notified-once flag (JSON, config dir): GlobalShortcuts v2 has no restore token, so "reuse across restarts" means identical re-binding without re-nagging — the honest reading of the plan acceptance, pinned in persist.rs docs.
- FALLBACK LADDER = plan-text-authoritative: portal -> paste-ready compositor snippets (hyprland/sway/gsettings-gnome) + documented guidance (KDE KGlobalAccel, generic). NO active KGlobalAccel D-Bus registration (KDE is portal-served per F14; the task brief defers to the plan text for the ladder).
- ACTIVE_SCREEN (u32::MAX) named sentinel for "active monitor" CaptureScreen: the todo-32 vocabulary has no under-cursor screen command; a documented sentinel defers resolution to the todo-12/18/35 executor instead of hardcoding screen 0 or widening the command enum mid-wave.
- ONE-TIME NUDGE: toast body names `flowshot settings` (textual deep-link); a clickable settings deep-link needs the todo-36 settings surface — recorded deviation in strings.rs docs. Gated by [daemon].notifications like every toast.
- DaemonOptions.shortcuts defaults DISABLED; the flowshot-daemon binary opts in via ShortcutOptions::production() — existing daemon tests/callers keep exact todo-32 behavior (broker tests manually set the persistence flag; untouched).
- Registration blocks Daemon::start up to a 30s budget AFTER sd_notify readiness + bus-name acquisition (a GNOME confirmation dialog must not delay service availability); timeout aborts the task and falls back.
- DesktopEnv REUSED from flowshot-capture (new daemon dep edge; zero new external crates) instead of a shortcut-local enum — single desktop vocabulary per F14; detection logic mirrored from capture-wayland (pure fn + env glue, Wayland gate kept: v1 is Wayland-only).

## 2026-09-26 (todo 33): SNI tray design decisions
- HAND-ROLLED SNI + com.canonical.dbusmenu on the daemon's existing zbus-4 connection: no SNI crate (ksni/tray-icon) in the workspace table OR the lock, root manifest orchestrator-owned; task brief sanctions hand-rolling, ksni's wire shapes used as the reference (Unlicense). Deviation from the plan's "ksni" wording recorded here + learnings.
- TRAY ICON = procedural token glyph (four corner brackets, the capture-region symbol) rendered straight to ARGB32-BE pixmaps at 5 sizes: todo-19's vendored set is editor-tool SVGs with NO app logo, and its atlas lives in flowshot-ui's OUT_DIR (consuming it = GPU-stack dep + PNG decoder in the daemon). IconName stays EMPTY (no installed FlowShot themed icon until packaging todo 41/42; an unresolvable name renders broken on some hosts) - packaging can flip to the themed name with a one-line property change. Attention color = red-600 constant (core palette has no danger token yet - todo 36 action).
- TRAY PERSISTENCE REASON is owned by REGISTRATION, not the config seed: DaemonState now seeds tray=false; set_tray(true) only after a successful RegisterStatusNotifierItem, released on watcher vanish (NameOwnerChanged loop re-registers on late arrival). An auto-spawned daemon with [daemon].tray=true but no host on the bus still idle-exits (task contract). Todo-32's `set_tray(config.daemon.tray)` seed line in Daemon::start removed.
- QUIT = tokio Notify seam in Daemon::run's select (ShutdownReason::Quit) - the org.flowoss.FlowShot bus vocabulary stays frozen at the todo-32 five methods + Invoke (no bus-level Quit; menu-only).
- ABOUT = version toast (NotificationRecord::About) until the todo-36 settings stack provides a real surface - recorded deviation from the plan's About dialog parity.
- HOST COMPAT: ItemIsMenu=true, Activate -> UnknownMethod (ksni-verified GNOME-appindicator/Plasma<6.4 menu fallback), SecondaryActivate -> quick region Capture, ContextMenu -> no-op (hosts render the D-Bus menu). Status idles at "Active" (Passive items get hidden); set_status seam + NewStatus emission exist but nothing drives them in v1.
- PER-MONITOR SUBMENU: stable ids SCREEN_BASE(100)+probe_index; refresh = AboutToShow(submenu) -> deduped background task (spawn_blocking CaptureThread per refresh, no resident wayland connection) -> revision++ -> LayoutUpdated signal; initial probe kicked at tray start; probe failure = disabled placeholder entry (headless-safe, infallible trait contract).
- DaemonOptions.tray seeds from config ([daemon].tray + [ui].accent_color) in the constructors - unlike todo-34's shortcuts (binary opt-in), the tray gate IS the config key; default config = off = pre-todo-33 behavior byte-identical.

## 2026-09-26 (todo 26): editor chrome design decisions
- CHROME-FIRST FUNNEL ORDER: the route funnel offers every pointer press/release to the chrome BEFORE the F27 editor chain (Qt child-widget parity - a press on the toolbar/wheel/panel never starts a draw stroke or region drag). A chrome-consumed press GRABS its release (implicit-grab parity, so the layer drag-reorder lands even when the cursor drifts); any Esc cascade step cancels the grab (a pending reorder must not land post-Esc).
- WHEEL SHOW MOVED FROM app.rs INTO THE FUNNEL: the editor's ColorWheel effect is applied by route_button itself (chrome is core-owned state) - the whole picker flow (open -> cascade stage 5 sync -> swatch pick -> hide) is headless-testable through inject_event, and the picker flag is set in the SAME event (the old shell-side show ran after sync_cascade, so stage 5 never armed). Action::ColorWheel's shell arm is now redraw-only.
- SYNC_CASCADE WRITES ALL FIVE STAGES NOW: 1/2/4 from the editor (todo 20), 3/5 from the chrome (panel effective visibility, wheel visible) - the todo-20 "raw seams for todo 26" are real producers; raw cascade_mut() pokes are futile for EVERY stage (editor/tests + selection case34 updated to drive real state, the case32/34 precedent).
- PANEL VISIBILITY = runtime Space toggle (DEFAULT HIDDEN, Flameshot parity: the panel starts closed, Space opens) AND the [editor].side_panel config gate (false = Space inert, paint skipped, stage 3 never occupied, presses fall through to the scene).
- UNIFIED SIZE SLIDER bound to the todo-20 dispatched slot (set_tool_size): one slider whose LABEL is the per-tool visibility table (side_panel::size_label - Thickness/Corner radius/Font size/Marker size/Block size/Counter size; None hides it for selection/move/invert). Replaces the previous worker's three mutually-exclusive sliders, two of which wrote through configure() (WRONG SEAM: configure re-projects ALL runtime size slots - a radius drag reset every wheel/digit adjustment).
- EDIT_TOOL_CONFIG PRESERVATION WRAPPER: panel toggles that must write tool config (arrow style/reverse, counter outline) go through configure() (the only tool-config seam) but FIRST write the wheel-picked draw color back into the config string (configure re-parses it - a stale hex would revert the pick) and RESTORE the active tool's runtime size slot afterwards.
- PANEL PROPERTY WRITES = editor/properties.rs (resize_selected/recolor_selected): exhaustive ToolObjectData maps onto the object's own field (thickness/width/stroke_width/corner_radius/font_size=slot+BASE_POINT_SIZE), Option-guarded so kinds WITHOUT the property (invert: no color no size; counter: geometry is todo-24-owned) record NO no-op journal unit - each write is exactly ONE undo unit through the todo-25 mutate_object funnel.
- LAYER ROWS BOTTOM-TO-TOP (plan wording), row index == paint z, so press-row/release-row maps straight onto move_layer(from_z, to_z) (ONE undo unit, todo 25). Drag = press + release (no motion tracking - motions stay inert for the scene because the press never reached the engine); a drop outside any row cancels the reorder but keeps the press selection.
- COLOR WHEEL = F27 spec: circular palette at equal angles from 12 o'clock, radius = 3*count + BUTTON_BASE_SIZE(32), swatch 24px, selected = 6px accent ring band hugging the swatch (stroke 6 wide centered +3 outside), rainbow slot trails the palette (click = debug-log seam, todo 27 owns the eyedropper flow), disc center clamped fully on-screen (min/max pair, never f32::clamp - degenerate outputs must not panic on inverted bounds).
- DRAW-COLOR PERSISTENCE = DrawColorSink callback (Box<dyn Fn(&str) + Send> on ChromeState): the lib stays pure, the binary layer (todo 35) / QA harness owns the TOML path (F27 "drawColor persists to TOML on change"; the plan's file-assert acceptance runs through the harness --draw-color-toml flag).
- TOOLBAR undo/redo buttons drive the todo-25 editor journal (they are editor ops, not W5 actions); copy/save/upload/pin/open-app/exit stay no-op debug-log seams per the plan MUST-NOT ("wired in W5 todos via callback traits"). New icons: chevron-up/chevron-down (raise/lower buttons), rainbow (wheel slot) - lucide ISC, the vendored set's license.
- SIZE HUD LEFT INERT: plan todo 26 does not include the notifier box (todo 20 shipped the timing-based digit reset without a visual); wiring it needs a size-change effect + tick deadline - recorded in issues for todo 38 instead of inventing scope.
- DEFERRED (recorded in issues): tooltips-400ms and the reveal/slide motion animations - they need hover-dwell tracking + a per-frame redraw scheduler the chrome does not have; no plan acceptance line covers them.

## 2026-09-27 (todo 17): magnifier design decisions
- CPU NEAREST-ZOOM TEXTURE over GPU sub-region sampling: the render image pipeline has ONE linear sampler (10x magnification smears pixel edges; Flameshot's QPainter fragment draw is nearest). CPU build also composites the todo-23 effect layer and yields the readout value for free. Per-frame cost trivial (289 samples + 115KB upscale + replace-under-fixed-id upload).
- UNIFIED CLAMP SAMPLING for both shapes (BORROW-MODIFIED): Flameshot's circle variant samples a black-padded screenshot — rejected because the readout must show the REAL pixel under the crosshair (wayshot --color equivalence) and padding fabricates black at screen edges. Square's clamp + arm-offset shift applied to both; documented in the module header.
- PHYSICAL-FIRST magnifier geometry: 170 PHYSICAL px at any scale (never 170 logical x DPR — the #4871/#4920 root-cause family Flameshot's mixed logical/physical drawPos lives in). Readout font scales (base_size * scale, HUD convention).
- TOGGLE KEY = L (lens): F12/Flameshot recognizedShortcuts has NO magnifier binding (source-fetched verification); unbound-key choice documented per the grid-F precedent, rebindable via the future [shortcuts] group.
- configure() projects BOTH magnifier config keys (visible + shape): a settings apply is authoritative over the in-session toggle (deliberate divergence from the grid's configure gap, recorded for todo 36 alignment).
- Readout format `#RRGGBB R,G,B` (uppercase hex, wayshot --color hex equivalence + the plan's RGB addition); bar width = max(widget width, 0.62em text estimate), flips above near the bottom edge.
- Grid/readout ink: grid = neutral gray 128/96 (todo-27 paint_grid precedent — palette has no neutral token), readout = contrast box @200 + white text (size-HUD precedent). Token-purity deviations recorded as todo-36 theming actions.

## 2026-09-27 (todo 18): launch-flow design decisions
- `--region at-cursor` SEMANTICS = the whole OUTPUT under the cursor becomes the initial selection (Preselect::OutputAtCursor). A size-less "center at cursor" has NO defined size anywhere in the spec (F12/plan/CLI help are silent); output-at-cursor is the established Amendment-#3 concept (`capture screen` no-arg) and gives the spanning overlay a per-monitor interactive starting rect. The CLI's RegionToken::AtCursor doc comment ("Center/anchor the preselection at the cursor") is ambiguous — todo 38 (mapping) + todo 40 (docs) confirm or remap; a different reading is a one-variant change in launch/resolve.rs.
- REGION-MEMORY PERSIST TRIGGERS = Accept (Enter OR instant release) AND Copy (Ctrl+C / double-click): both are capture-completing gestures and `capture last` repeats the last CAPTURED region. Single seam: launch_persist at the funnel exits (route_button main path + route_key); the chrome-consumed early-return branch skips it (chrome has no capture gestures today; todo 38's toolbar copy/save wiring routes through the funnel or calls launch_persist explicitly).
- CLAMP SEMANTICS SPLIT: explicit coords + last-region = INTERSECTION with the layout (honest crop — off-layout pixels cannot be captured; zero overlap = no preselect + warn, never an error); cursor-centered = size-preserving SHIFT into bounds (the acceptance's "dimensions EXACTLY WxH" contract; an oversized request crops to bounds). Both reuse the todo-16 engine math (fit_into_bounds pub(crate) + clamp_region_to_layout) — one clamp, two callers.
- MINIMUM-AT-SEED: any preselect side < 10x10 (the todo-16 engine minimum) expands at seed time, so a seeded rect always satisfies the engine invariants (a 5x5 seed would otherwise snap on the first nudge/resize).
- INSTANT = fire-once arming: the FIRST left release that LEAVES a selection emits Action::Accept and disarms; a release that CLEARS the selection (outside click on a preselect) keeps the arming for a later drag; any Accept (Enter included) disarms (one accept per session). Right/middle releases never accept. "No editor step" is inherent: the release fires before any annotation, and Action::Accept is already the binary layer's export trigger.
- AwaitFirstMotion is ONE-SHOT: the first motion is the todo-12 resolution point; a motion position outside every output (layout gap) degrades to no preselect with a warn — never retried per motion (a preselect popping in mid-session on the tenth motion would be worse than none).
- LaunchRequest::default().save_last_region = FALSE (a bare Default launch persists nothing — safe for tests/harness); the binary layer projects [capture].save_last_region (config default true). RegionSink follows the DrawColorSink callback pattern: the lib stays file-pure, the example/binary owns the TOML path.
- LaunchState resolution order: launch() stores the request + resolves immediately IF a layout is installed (headless cores, QA harness); the production path resolves at the spawn hook (apply_launch after router install on Resumed). A later launch() supersedes an earlier request (last writer wins, seeded flag reset).

## 2026-09-27 (todo 36): settings-surface design decisions
- EGUI-WINIT DROPPED — the plan's pre-recorded fallback executed: egui-wgpu 0.28.1 ↔ wgpu ^0.20.0 MATCHES the workspace pin (cached-registry-verified), but egui-winit 0.28.1 requires winit ^0.29.4 (no 0.30 flag in its manifest) and every newer egui needs wgpu 22+ (bump forbidden) → NO release pairs (wgpu 0.20, winit 0.30). Manual feed: settings::input accumulates winit 0.30 WindowEvents into egui::RawInput, settings::keymap mirrors egui-winit 0.28's conversion tables 1:1 (logical-first + physical fallback per egui#3653, numpad merged, printable-text filter with private-use-area rejection, command=ctrl on Linux). Root table's egui-winit pin stays unused (orchestrator may drop).
- SEAMS OVER DEPS (purity gate holds — no rfd/ashpd/zbus in flowshot-ui): PathPicker callback (binary layer wires rfd; None disables Browse), AppliedCallback (binary layer emits ConfigChanged + re-projects EditorState::configure/chrome/daemon), ClipboardBridge (Ctrl+C/V; None = inert), system ThemeMode injected via options + SettingsHandle::set_system_theme live push (binary layer owns the ashpd Settings portal query), app_id via the pins::WindowCustomizer precedent. Apply = validate → Config::save (migration-safe; load already migrated) → callback → mark_clean; failure = banner, never panic.
- LIVE-APPLY = immediate-mode style re-projection: every frame rebuilds egui::Style from DesignTokens + the IN-EDIT [ui] config (accent drag re-themes the window before Apply). Pixel-proven offscreen (edited_accent_changes_the_rendered_theme).
- OFFSCREEN QA PATH = render_offscreen(): SettingsSurface + egui_wgpu::Renderer into a plain texture (Renderer::render takes ANY render pass — no SurfaceTexture needed), Rgba8Unorm gamma-space target so readback bytes land in theme byte space (paint() clears sRGB-format targets with the linear panel fill, gamma-space targets with the sRGB bytes directly — the format-aware clear is load-bearing for pixel asserts). GPU-skip pattern copied from tests/parity.rs.
- TOOLBAR ORDER EDITOR = ↑/↓/✕ + add-from-vocabulary combo (egui 0.28 has no drag-list widget; recorded deviation from the plan's "drag-list" wording). Vocabulary = ToolKind::ALL ids + the chrome action ids (copy/save/pin/upload/undo/redo/open-app/exit).
- SHORTCUT RECORDER writes through the todo-25 seams (rebind/rebind_undo_redo/rebind_z_order; new read accessors undo_key/redo_key/z_keys on ToolShortcuts). Keys without a physical KeyCode (Colon/Pipe/Questionmark) are rejected WITHOUT disarming. Persistence still blocked on the core [shortcuts] group (issues.md 2026-09-26) — rebinds are model state for the binary layer; global portal-shortcut rebind needs a daemon seam (todo 38).
- SAVE-ACTION SET = ordered 7-variant checkbox set rebuilt in canonical enum order on every toggle (byte-stable TOML); action_label's exhaustive match fails the build if core grows a variant.
- WINDOW = normal decorated window on the existing runtime shape (require_display_server, run_app + fatal extraction, Arc<Window> → Surface<'static> per the pins precedent); repaint via ViewportOutput.repaint_delay → ControlFlow::WaitUntil (egui 0.28 removed FullOutput::repaint_after) — idle = zero CPU. New gpu::configure_opaque_surface shares one policy core with configure_overlay_surface (Opaque vs PreMultiplied alpha is the only difference).
- GRID ALIGNMENT DONE (todo-17 issues ACTION for todo 36): EditorState::configure() now re-projects grid_visible from config.editor.grid — the magnifier pattern; a settings apply is authoritative over the session toggle.

## 2026-09-27 (todo 37): capture launcher dialog design decisions
- EGUI, NOT THE WIDGET LAYER: the dispatch brief said "NOT egui - egui is the settings-only exception", but the plan todo 37 text (which the brief itself declares AUTHORITATIVE) says "small egui window on the todo-36 embedded stack" and "quick - small egui dialog on finished settings stack" -> plan wins; the egui exception (draft D8(b)) now covers exactly TWO windows: settings + launcher. Module docs updated ("the overlay and editor never touch egui" invariant unchanged).
- EGUI_HOST EXTRACTION: the todo-36 embedded stack moved from settings/ to a shared crate-private egui_host/ module (input.rs + keymap.rs verbatim moves; SettingsSurface generalized to EguiSurface with frame_with<A: Default>(style, show) - CentralPanel::show's InnerResponse.inner feeds A directly; theme.rs moved; present.rs = scale_to_ppp/points/cursor_icon). Settings keeps its public API (ClipboardBridge/ThemeMode/style/fonts/parse_hex_rgb re-exported from egui_host; SettingsSurface name RETIRED - no external consumer existed). This materializes the plan's "on the todo-36 embedded stack" literally: ONE stack, two windows.
- SURFACE = PLAN-EXACT (capability parity MUST-NOT): target dropdown ("Manual region" + live-probed outputs) + geometry TextEdit WxH[±X±Y] with inline validation + delay DragValue (ms, 0..=u32::MAX) + Capture/Cancel. NO fullscreen/last-region/mode-picker entries (the brief's "region/fullscreen/per-monitor/last-region" prose defers to "the plan's surface", which is the manual-coordinate dialog only).
- DISPATCH = TYPED REQUEST SEAM (LaunchCallback, the AppliedCallback pattern): LauncherRequest::Region{geometry: RegionGeometry, delay_ms} | Screen{screen: probe-index u32, delay_ms}. flowshot-ui stays pure (no zbus/probe deps); todo 38 maps Region -> Capture(CaptureRequest{region: Some(to_token()), delay_ms}) and Screen -> CaptureScreen(screen). Capture closes the window; Cancel/Esc/close-button close WITHOUT dispatch (the CLI's exit-3 user-cancelled class).
- MONITOR INDEX AUTHORITY = the todo-6 probe order (MonitorProbe seam returns Vec<OutputInfo>; entry.screen = probe index) - the SAME indexing the todo-33 tray submenu and the daemon's CaptureScreen(n) use. winit's available_monitors() was REJECTED as the source (order not guaranteed to match the capture probe; wrong index = wrong output captured). Labels reuse the tray's "Screen {n}: {name|connector}" format.
- ENTER = DEFAULT-BUTTON DISPATCH (Enter while the geometry field holds focus dispatches when request() is Some; checked BEFORE widgets render so the TextEdit's own Enter-surrenders-focus handling cannot eat it). Reason: egui 0.28's Tab focus ring measured UNSTABLE in this dialog (see learnings) - Enter-in-field is the deterministic keyboard path + standard dialog convention, not an extra feature. Focused-button Enter/Space (egui native) still works when focus lands there.
- TEST-DRIVE INJECTOR ENTERS AT THE egui::EVENT LAYER (LauncherInput::{Text,Key,Pointer} -> egui Events queued on the production InputState): winit 0.30's KeyEvent has PRIVATE fields (WindowEvent::KeyboardInput is not constructible), so the pins-style WindowEvent synthesis is impossible; the pins precedent (domain-level injection converted at the app layer) applies one layer down. The bypassed winit->egui conversion table is the settings keymap's independently-tested territory. Pointer injection (physical px -> points via pixels_per_point) drives the real ComboBox for the monitor-mode QA.
- QA EXECUTION IN THE HARNESS (dev-dep flowshot-capture-wayland, the flowshot-actions precedent): the plan acceptance needs a SAVED 100x100 image == grim oracle, but the production capture executor is todo 38 - so examples/launcher_dialog.rs executes the typed request itself (ICC capture_region / capture_output_named + delay sleep + PNG save). Result: acceptance closed LIVE today (exact-pixel-match 1.000000 vs grim crop), and the harness doubles as the todo-38 reference wiring.
- OFFSET-LESS WxH ACCEPTED (CLI --region grammar parity): executor centers at cursor (todo-18 semantics); the harness resolves the cursor through the F13 ladder (resolve_cursor_pos). RegionGeometry mirrors the CLI's RegionToken::Rect field-for-field; to_token() reproduces the wire format (roundtrip-tested incl. i32::MIN via unsigned_abs).

## 2026-09-27 (todo 38): executor + process-model decisions
- EXECUTOR HOME = flowshot-daemon::execute (library): both the CLI one-shot
  path and the daemon's ExecutingSink call the SAME pipeline (cli depends
  on daemon; the reverse edge would be circular). Modules: backend (ladder
  runner + RUNTIME fallthrough), direct (window-less captures), overlay
  (child session + configure_core), post (actions/stdout/region-memory/
  persistence-reasons), pin/launcher/settings (child sessions), invoke
  (daemon-side argv parser), session (child contract), headless
  (test-drive execution mode).
- WINDOW SESSIONS = CHILD PROCESSES (`<exe> session --spec`): forced by
  winit 0.30.13's one-EventLoop-per-process guard (issues.md). Parent runs
  post-capture => daemon-owned clipboard (todo 28), single pin registry,
  crash isolation. Image handoff = temp PNG (lossless) + result JSON.
- SINGLE-SESSION GATE: one active window session per daemon
  (AtomicBool in session.rs) = the dropped allowMultipleGuiInstances
  semantic; window-less captures are ungated.
- INVOKE PARSING: the daemon hand-parses the FROZEN forwarded subset
  (capture full/screen[spec] + modifiers, pin, color) — it cannot reuse
  the CLI clap surface (dependency cycle). Grammar mirrors the CLI 1:1
  with typed usage errors; the shared-core parser remains an orchestrator
  follow-up (todo-37 note).
- LAUNCHER DISPATCH: daemon mode forwards over the bus (Region ->
  Capture{region token} = interactive preselect per the todo-37 mapping;
  Screen -> Invoke["capture","screen",n,"-d",ms] = delay honored, the
  todo-37 OPEN question decided: no wire extension, nothing dropped);
  one-shot mode captures the typed geometry DIRECTLY in the child (a
  second event loop is impossible; the todo-37 harness-proven semantics).
- EDITOR SAMPLING FRAME = stitched scale-1 composite of the whole layout
  (every output sampleable; logical-resolution sampling on scale>1
  outputs documented) over per-output first-frame (blind beyond output 0).
- CURSOR LADDER SKIP: the extra ICC cursor round-trip is only paid when a
  cursor-dependent preselect exists (offset-less --region / at-cursor) —
  perf budget fix (capture_ready 122-178ms -> 71-88ms dual).
- `capture screen` NO-ARG with unresolved cursor = FIRST output + loud
  warn (todo-18 open question decided: deterministic, never a failure).
- STDOUT MODES: --raw EXCLUSIVE (its bytes own the stream);
  --print-geometry ADDITIVE (prints, then the action set runs); both
  together = usage error (exit 2, executor-side; the CLI parse surface
  stays as todo-35 pinned it).
- EXPORT RENDER: the live shell renders through each window's EXISTING
  renderer (Backdrop::upload_for DRAINS the CPU pixels — texture residency
  forces this) with BGRA->RGBA normalization by renderer.format(); the
  headless path uses a fresh Rgba8UnormSrgb renderer (the
  verify-offscreen pattern). ONE composite implementation
  (completion::composite_selection) under both.
- PERF 1080p = DOCUMENTED-DEGRADED (not claiming green): composed
  ~180-210ms vs 150ms budget, NVIDIA/Vulkan bring-up dominated;
  remediation designed (07-perf-budgets.md), live re-measure todo 41.
- CALL_TIMEOUT (daemon stub tests) 15s -> 45s: the recorded remedy for
  the load-sensitive deadline family, observed again this run.

## 2026-09-28 (todo 42): gate/audit decisions
- PURITY GATE = scripts/purity-gate.sh (shell, CI step in ci.yml before fmt) + scripts/purity-allowlist.txt. Gated: flowshot-core/flowshot-capture/flowshot-ui src/** + their Cargo.toml dep sections. Banned families: wayland (client/protocols/backend/scanner/sctk/calloop/wl-clipboard-rs), x11 (x11/x11rb/xcb/xcap/xkbcommon), dbus (zbus/zvariant/ashpd/ksni/dbus/notify-rust), unix-FFI (nix/rustix/libc/memfd), pipewire/libspa, fontconfig, flowshot-capture-wayland, flowshot-actions, winit PLATFORM EXTENSIONS (winit::platform::*, *ExtWayland/*ExtX11/...), cfg(target_os/target_family/target_arch/target_env/target_abi/unix/windows/macos). winit proper ALLOWED (D1 cross-platform seam; plan gate list = "wayland/x11 imports or cfg(target_os)"). D8 egui settings module NOT purity-exempt (stack exception only) - scanned clean, no exclusion needed. Allowlist has exactly 2 entries: ui's dev-deps flowshot-actions + flowshot-capture-wayland (QA-harness composition, examples/tests only). tests//examples/ out of gate scope (dev surfaces).
- deny.toml FINALIZED (42d/Metis #9): Unicode->Unicode-3.0 (cargo-deny 0.20 rejected the shorthand - CI deny step was red since todo 1), +OFL-1.1, +CC0-1.0/NCSA/Apache-2.0 WITH LLVM-exception/CDLA-Permissive-2.0 (each forced by the pinned transitive graph, GPL-compatible, provenance in file comments), epaint LicenseRef-UFL-1.0 per-crate exception (embedded egui fonts, never rendered - vendored Inter replaces), [advisories] per-ID ignores RUSTSEC-2024-0436/2026-0206/2026-0192 (unmaintained, transitive, no upgrade path; new advisories still fail), [bans] multiple-versions = "warn" NOT "deny" (121 transitive dup entries; only root-actionable = zbus 4->5 consolidation, STOP-and-reported; flip to deny after).
- NOTICES lives at repo ROOT (common OSS convention), generated from `cargo deny list -f tsv` + vendored LICENSE files (never from memory): 643 third-party packages / 27 license groups / OFL-1.1+ISC+MIT full texts / exact Feather-derived icon subset (11 of 24, computed via comm). Project license stated as GPL-3.0-or-later per README+workspace (the crate-manifest MIT/Apache mismatch is reported as finding F-1, not silently adopted).
- Porting roadmap = docs/porting-roadmap.md (standalone) + ADR-006 addendum link (task allowed either; ADR keeps its summary, doc carries the >=3-crates-per-phase detail + F20 conformance table + F24 red flags + crates.io spot-check record + init-systems phase). BackendKind roadmap-variant deviation recorded: shipped as documented non-constructible placeholders (is_roadmap() + typed NoBackendAvailable) instead of the plan's "cfg-excluded" wording - functionally equivalent, no cfg needed.
- Unsafe allow-list = { flowshot-capture-wayland } ONLY, documented in ADR-006 + roadmap doc; exemption currently UNUSED (zero unsafe blocks workspace-wide; wl_shm readback = plain file I/O via nix safe wrappers). No SAFETY comments were missing; no src edits made.

## 2026-09-28 (todo 41): motion/polish decisions
- MOTION OWNERSHIP: chrome transitions live in ONE ChromeMotion owner (chrome/motion.rs) ticked
  from OverlayCore::tick (about_to_wait cadence, rising-edge reveal detection); grip hover lives
  in the selection engine (GripMotion) fed by the funnel on EVERY motion regardless of editor
  consumption (handles stay hoverable under an active tool); pin zoom easing lives in PinState
  (pins/anim.rs) with the shell's own about_to_wait pacing. Frame pacing (FRAME_INTERVAL=16ms)
  is centralized in OverlayCore::wake — motion owners expose raw settle deadlines only.
- REDUCED-MOTION SWITCH = UI-side seam (OverlayCore::set_motion_reduced + PinBehavior.
  reduced_motion), NOT a config key: the plan's QA scenario says "reduced-motion config" but the
  todo-41 dispatch forbids flowshot-core edits — the [ui].reduce_motion key + settings toggle is
  a recorded follow-up (issues.md). The UI leg is complete: unit + pixel-identical oracle pair.
- HIT-TESTING ON FINAL GEOMETRY during transitions (interaction leads the ≤180ms visual) —
  keeps the chrome behavior suite green and avoids mid-animation unclickable states.
- PIN ZOOM: window resize instant, CONTENT eases (compositor min==max mechanism can't animate;
  anchor convergence pos(e)=cursor−e·delta proven). Pinch preview bypasses easing (direct
  manipulation must not lag).
- TOOLBAR/WHEEL/PANEL now draw token shadows (shadows.medium/large) — D8(c) "shadows FROM
  tokens"; the wheel's square-bounds shadow at radius=half-side rounds into the disc silhouette
  (rect shadow primitive fits the circle).
- IMAGE ALPHA added to the renderer vocabulary (ImageCommand.alpha, image_faded()) as the
  icon-fade enabler — a motion primitive, not a behavior change; parity reference updated in
  lockstep.
- QA BUNDLE = new examples/qa_bundle.rs on the EXTRACTED production frame builder
  (flowshot_ui::build_overlay_frame, shared with render_window) + new launcher::render_offscreen
  (settings pattern) + existing settings_offscreen/pin_window harnesses. Notification shot =
  honest N/A (compositor-rendered surface).

## 2026-09-28 (F1 remediation): commit-strategy deviation accepted

Plan rule: "ONE commit per todo (42 commits + in-todo fix commits), message
exactly as each todo's Commit line." Actual: 34 commits.

**Deviation details:**

- Combined commits (bodies carry `Refs: todo N` — traceability preserved):
  910163d (11+19), 0f4d17b (16+31+config-gap), d5b1777 (20+30), 6779c1b
  (21+32), 518b511 (22+34), 03681bd (23+33+35), 8716cc5 (36+40).
- No dedicated commit (work rides inside another todo's commit): 14, 15, 28,
  29, 40 (in 418d823 / 848f0e5 / 8716cc5).
- Systematic scope drift `editor:`→`ui:` (todos 20-27) and `shell:`→`ui:`/
  `daemon:` (33-37); todo 42 lost its `(gates)` scope.

**Rationale:** Parallel-lane execution made per-todo atomic commits
impractical from wave 3 onward. Multiple todos were being worked
concurrently by different workers, and their changes interleaved in the
working tree. Combining related work into single commits preserved atomic
gate-before-commit semantics (every commit body carries `Refs:` lines for
traceability) while avoiding broken intermediate states.

**Acceptance:** This deviation is recorded as an accepted plan deviation.
Every todo's work exists in history and is traceable via `Refs:` bodies.
The one-commit-per-todo + exact-message rule was not honored from wave 3
onward, classified as a process deviation, not an acceptance-criteria
failure.

## Wheel-on-rectangle = corner radius (F27, re-confirmed 2026-10-02)
Task asked whether wheel-on-rect should adjust stroke thickness instead of the corner-radius slot. DECISION: keep the todo-20 dispatch (rect slot = `[tools.rectangle].corner_radius`). Grounds (fetched Flameshot sources, not memory): `flameshot.example.ini` documents `drawRectangleSize` as "Last used size for Rectangle rounded corners"; `confighandler.cpp setToolSize` routes TYPE_RECTANGLE to `setDrawRectangleSize`; `rectangletool.cpp process()` reads that one value for the rounded-path radius (its QPen is vestigial under `fillPath` - Flameshot's rect is FILLED). So the radius IS the rectangle's size semantic upstream. FlowShot's rect is a stroked outline whose pen stays the persisted `[editor].draw_thickness` (todo-20 split, unchanged). The defect was invisibility, fixed via the size-notifier HUD + the hover dot now following the dispatched size (Flameshot's rect mouse-preview sizes from the same `onSizeChanged` value). The shape.rs module doc previously misstated Flameshot ("never the pen width") - corrected with the citations.
