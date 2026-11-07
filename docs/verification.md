# Verification policy

How FlowShot is verified, and how to read the claims this project makes. The
short version: every feature is verified after it lands, the strongest
feasible evidence class is used, and every claim carries its class so that
"works" never silently means "compiled once."

## Tests-after policy

Every module lands with its tests in the same change: unit tests for pure
logic, property tests for algebra (geometry, scene invariants), golden
fixtures for rendering parity (the real wgpu pipeline against an independent
tiny-skia rasterizer, dev-only), private-bus stubs for D-Bus wire contracts,
and HTTP stubs for upload. The gates are `cargo fmt --check`, `cargo clippy
--workspace -- -D warnings`, `cargo test --workspace`, and `cargo deny check`;
CI runs the same set. A behavior is not "done" when it compiles; it is done
when a test that would fail without it is green.

Headless-green is not live-green for GPU and windowing code. Every GPU or
window feature additionally gets a timeout-bounded live run on the real
compositor before it is called verified (a lesson bought with a live-only
2048-px texture-limit panic and an NVIDIA EGL teardown segfault, both fixed
because the live run caught them).

## Live-session QA and its class labels

Live QA runs on two machines: the primary development box (Hyprland
0.56.2, NVIDIA GPU, two monitors at scale 1, plus synthetic headless
outputs for scale-2 and rotated fixtures) and a second i3/X11 laptop
(single eDP-1 2880x1620, derived scale 2.25) used for the X11 capture
path; every LIVE-verified claim names the machine its bundle ran on.
Every claim in FlowShot's docs and evidence
carries one of these classes:

- **LIVE-verified**: exercised on the real session, with the observed values
  recorded (pixel oracles via grim, `hyprctl`/`busctl` reads, logs).
- **Source-verified**: the behavior is pinned against the other party's
  source code at a recorded commit (wire contracts, protocol quirks), but
  the path was not run live on that desktop.
- **Unit-level**: covered by tests only; the live path is explicitly not
  claimed.
- **Hardware-gated**: cannot run on the QA machine the check is scheduled
  for at all (neither box has a KDE or GNOME session, no touchscreen; the
  X11 laptop is single-panel, so multi-monitor checks are gated there).
  Never claimed, scheduled for suitable hardware.
- **Deferred**: runnable on the QA machine but queued, usually because it
  needs a visible window or an in-flight component (see the batch mechanism
  below).

Docs like the per-desktop setup guides are written to this discipline: if a
sentence has no class marker on Hyprland, it is live-verified; on other
desktops the class is stated at the top of the page.

### Live-run choreography

Unattended live runs are timeout-bounded and self-reversing: transient
compositor state (headless outputs, runtime binds, window rules) is created
and removed in the same script, and artifacts are namespaced so cleanup
globs cannot eat inputs. Input is driven through the UI crate's test-drive
injection seam (synthetic events through the production routing path) or,
when the tooling exists, `wtype`/`ydotool`. Screen assertions use grim as
the independent oracle; pixel comparisons are computed against the known
render pipeline (linear-light blending, premultiplied alpha) rather than
hand-tuned thresholds.

## Evidence trail convention

QA evidence lives in `.omo/evidence/` (local to the working repo, not part
of the shipped product): one bundle per change, recording the environment,
the exact commands, the observed values, the gate outputs, the verification
class of every claim, and honest records of deviations and foreign defects
found along the way. Evidence files record observed values inline because
temp artifacts are transient.

### Landed bundles

- **Open-path instrumentation and the two render-side wins it revealed**
  (2026-11-07, branch `perf/ui-open-latency`, **LIVE-verified** for the timings
  — measured on the primary box (Hyprland 0.56.2, NVIDIA RTX 5060 Ti), 2 windows,
  `empty_overlay` built `--features 'test-drive perf-trace'`, release, bounded
  `timeout 25` runs that exit 124 — and **Unit-level** for the shared-kit contract,
  which executes on the measuring machine and skips with a message on a host without an
  adapter). What landed: `flowshot-ui/src/startup.rs` (`StartupClock`, always
  compiled, `target: "flowshot_perf"`, one monotonic anchor at `OverlayApp::new`)
  wired at `handler/mod.rs` (`perf.windows_ready`, `perf.gpu_instance`,
  `perf.gpu_ready`, `perf.renderers_ready`) and at `app.rs`'s `render_window`
  (`perf.ui_ready` only when the present succeeded); `render/fonts.rs` (one lazy
  `FontSystem` per process thread behind `with_font_system`, with a re-entrancy
  guard that falls back and logs instead of panicking); `render/device_kit.rs`
  (`DeviceKit` — pipelines, bind group layouts, samplers, the MSAA sample count and
  the texture limit built once per format and cloned into each window; the handler
  holds one kit per format, so a CPU-lane child never pays for the render
  pipelines, and the `Presenter`'s pipeline stays per window on purpose).
  Pinned by tests that landed with the modules: `startup/tests.rs`
  `milestones_emit_in_the_documented_order`,
  `ready_milestone_fires_once_across_repeated_frames`,
  `no_readiness_reported_before_any_present`; `render/fonts.rs` inline tests
  `the_store_is_not_built_until_the_first_shape` and
  `one_cell_per_thread_so_every_caller_aliases_the_same_store` (each on its own
  thread, so they do not depend on test order); `render/device_kit_tests.rs`
  `one_kit_is_reused_by_every_window_instead_of_rebuilt`, which is falsifiable by
  construction — it asserts the equalities AND a control kit built on the same
  device is a different pipeline and sampler, and that the glyph atlas bind group
  stays per window.
  Observed values (same harness, same box, legs are deltas of the cumulative
  `elapsed_us`, anchor = `OverlayApp::new`). Before, recorded 2026-11-06:
  warm 1.931 / +66.075 instance / +108.424 device / **+62.387 renderers** /
  +13.685 frame+present => 260.6 ms, repeats 268.3 and 283.0; cold first run 7.179 /
  +202.242 / +165.439 / **+445.239** / +4.785 => 824.9 ms. After, clean runs taken
  with no other cargo process running: 8.259 / +201.229 / +149.184 /
  **+16.999** / +17.608 => 393.325 ms; 1.783 / +66.846 / +107.877 / **+17.391** /
  +39.679 => 233.580 ms; 1.381 / +55.486 / +116.230 / **+17.054** / +19.413 =>
  209.764 ms. So the renderers leg is 17.0-17.4 ms against the 62.4 ms warm
  baseline, and the warm child total is 209.8-233.6 ms against 260.6-283.0 ms.
  Two caveats recorded as caveats: the before side is the 2026-11-06 measurement on
  the same harness and box, not a same-session A/B (a controlled A/B would need
  selectively reverting only the font and kit changes, since the milestones live in
  the same handler file as the kit code); and six runs taken earlier in the same
  measurement pass are not quotable — they ran while `cargo clippy`/`cargo test` competed for the
  cores and came out at instance 0.51-2.35 s, device +2.2-6.9 s. The lazy store
  moves the fontconfig load out of renderer construction into the first frame (the
  frame+present leg rose to 39.7 ms on one clean run), so what is saved is the
  duplicate work across windows, not the font build itself.

- **Pencil and filled rectangle as toolbar tools, plus the v3->v4 toolbar
  migration** (2026-10-28, **Unit-level** - no live-session run is claimed:
  nobody drove a real capture through either tool, and the visible-overlay class
  stays deferred under the policy below). What landed in `88b7cc6`:
  `ToolKind::RectangleFill` (id `rectangle-filled`) as its OWN toolbar button
  beside the outline rectangle, the freehand `pencil` id leading the drawing
  tools, `icons/square-filled.svg` for the atlas, the default toolbar grown to
  14 buttons, and a `CONFIG_VERSION` 3->4 step (`migrate_v3_to_v4`) that appends
  both ids to a user-pinned `ui.toolbar_buttons`. Pinned by tests that landed
  with it: `v3_config_gains_the_v4_toolbar_tools_on_migration` (a v3 table with
  a pinned 12-button list gains both ids) and
  `v4_toolbar_migration_is_idempotent_and_never_pins_a_missing_key` (a second
  run changes nothing; a config with no `toolbar_buttons` key never materializes
  one) in `flowshot-core`; `filled_rect_commits_a_solid_rect_under_its_own_kind`
  in `flowshot-ui` (the committed object reports `type_id() ==
  "rectangle-filled"`, round-trips through `ToolKind::from_id`, shares the
  rectangle scene type with `filled == true` and `corner_radius == 0.0` /
  `stroke_width == 0.0`, while the outline sibling stays `"rectangle"` with
  `filled == false`). `ui/tests/parity.rs` gained a `rect-filled` golden row
  (drag 120,120 -> 520,360 under `Config::default()`) on the plain assert rather
  than the edge-masked one, because `fill_rect` edges are unmasked; per the rule
  recorded below that row SKIPS on any host without a GPU adapter, so this
  bundle claims NO GPU comparison for it. Gate evidence is CI, not a local run:
  the feature push (`88b7cc6`, run 40713522594) passed the purity gate,
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings` and
  `cargo test` on Linux and every Windows step (scoped clippy, scoped tests,
  release build of cli+daemon), and failed ONLY at `Check dependencies` on
  RUSTSEC-2026-0382/-0383/-0386. `49068df` (Cargo.lock only: tokio
  1.53.1 -> 1.54.1, rustls-webpki 0.103.15 -> 0.103.16, webpki-root-certs
  1.0.9 -> 1.0.11) cleared it - run 40716996231 green. The docs pushes after it
  are green too: `ea8438d` + `b4a6b3d` in run 40739414577 (GitHub Actions runs
  the head commit of a push only, so the README commit has no run of its own),
  and `4b7a6fb` in run 40747262797 (Linux 5m24s, Windows 18m21s, both jobs
  `success`). Workspace test total measured that day: 1,537; rustdoc warnings: 0.
- **CPU raster fallback** (2026-10-10, **Unit-level** - the live fallback path
  is explicitly NOT claimed: neither QA box has a GPU-less or OOM-starved
  desktop, so the policy branch never took the CPU path in a live session).
  What IS covered, by tests that landed with the modules: the policy is pure
  data and pinned at both measured cases (`software_path_matches_the_measured_cases`
  - 4480x1440 at 213 MB takes the CPU path, 2560x1440 at 116 MB keeps the GPU
  path, a software device class takes it at any extent, and the budget
  boundary flips between 4.0 M and 4.1 M pixels); the CPU renderer's own
  contracts are pinned pixel-exact (`the_opaque_fill_lands_in_linear_light`,
  `clip_punches_only_the_clipped_region`, `invert_complements_painted_pixels`,
  `a_1_to_1_blit_copies_the_source_bytes`, `a_missing_texture_draws_the_magenta
  placeholder`, `degenerate_geometry_paints_nothing_and_never_panics`). A QA
  override (`adapter::RENDER_PATH_OVERRIDE_ENV` = `FLOWSHOT_RENDER_PATH`,
  `software`/`gpu` byte-exact tokens) masks the policy so the CPU path is
  reachable on a healthy GPU desktop; its rule is pinned by
  `render_path_override_names_only_the_two_tokens`, and activation logs a
  `warn` line naming the window, extent, and whether the policy or the
  override chose it. A label-only status pill (`chrome/status`, the
  `StatusChip` widget) rides the SELECTION: it hugs one of the four slots
  just outside the selection rect on a medium margin, preferring the slot
  diagonally opposite the aid cluster and skipping any slot the toolbar plate
  or the aid chips own; it states the rasterizer presenting that window —
  `GPU` quiet when all is well, accent-tinted `CPU fallback` when the
  fallback took over — and it is GONE (never clamped) when no slot fits or no
  selection exists. Its seven contracts (`the_pill_hugs_the_selection_below
  _trailing`, `the_pill_walks_to_the_next_free_slot`,
  `the_pill_is_gone_when_no_slot_fits`, `the_pill_is_gone_without_a_selection`,
  `the_pill_scales_with_the_output`, `the_pill_names_the_path_in_both_states`,
  `the_pill_is_one_body_and_one_label`) pin the placement walk, both reads,
  both gone rules, and that the pill is one fill plus one label with no key
  plate and no wash.
  The whole-workspace gate (`cargo fmt --check`, `cargo clippy --workspace
  --all-targets -D warnings`, `cargo test --workspace`, the purity gate, the
  deny check) passes with 683 flowshot-ui lib tests green — run with `-j 2`
  on the build host because a 29 GB resident local model server had the box
  swapping at load 255, which starved the default 16-way link stage to 0 %
  CPU for 39 minutes; the cap is a host condition, not a gate relaxation.
  Cross-rasterizer pixel parity of the CPU path against the GPU renderer is
  now **Unit-level covered through a real GPU**: `ui/tests/parity.rs` renders
  the same fixtures through wgpu offscreen and through the shipped
  `SoftwareRenderer` and applies the existing policy - `the_cpu_rasterizer_matches_the_gpu_on_flat_fills`
  and `the_cpu_rasterizer_matches_the_gpu_on_image_blits` are byte-exact
  (max_diff 2/255), `the_cpu_rasterizer_matches_the_gpu_on_rounded_shapes`
  lands at within2 99.61 % with max_masked 93/255 inside the coverage-vs-MSAA
  budget the policy already allows. The suite runs for real on this machine
  (19/19 green) and skips with a printed message where no adapter exists.
  Perf of the CPU path is **Unit-level measured** through the repo's own
  `perf_storm` harness, which gained a `--software` mode that renders the same
  production scene through `SoftwareRenderer`: 11.9 ms per both-window frame at
  1920x1080 + 2560x1440 with 16 committed objects, against 298 ms before the
  color and addressing work - the capture decode moved to registration (298 ->
  55 ms), the 1:1 blit got integer texel addressing (55 -> 25.6 ms), and the
  full-frame opaque backdrop over a still-transparent framebuffer became a
  per-row memcpy (25.6 -> 11.9 ms).

- **X11 Phase A headless capture** (2026-10-04, **LIVE-verified** on the
  i3/X11 laptop; bundle `.omo/evidence/x11-phase-a/`, local to the QA
  laptop's working repo - observed values inlined here per the convention
  above). Ten checks, all PASS:
  full capture pixel-cross-checked against an independent
  `import -window root` oracle, `--region WxH+X+Y` geometry pixel-exact
  against a crop of the full frame, screen by connector name and by index,
  `--raw` stdout PNG, `--print-geometry`, daemon-owned CLIPBOARD with INCR
  (a 1.08 MB offer served after the CLI exited and gone after the daemon
  was killed), the exit-code matrix (0/1/2/4), SHM fd-passing vs plain
  timing (38 ms vs 80 ms), the `-d` delay path, and daemon idle-exit
  lifecycle. Per-check values and environment dump: `index.txt` in the
  bundle.
- **X11 Phase B interactive UI** (2026-10-05/06, **LIVE-verified** on the
  second QA machine - the i3/X11 laptop, single eDP-1 2880x1620 @ derived
  scale 2.25; binary under test hash-identical to HEAD `8961497` at bundle
  time). Multi-monitor overlay spanning and the Wayland row of the
  exit-code matrix are **Hardware-gated** on that single-panel machine and
  are NOT claimed. Bundle: `.omo/evidence/x11-phase-b/` (local to the QA
  laptop's working repo; the observed values are inlined here per the
  convention above). Twenty interactive checks plus four rider proofs, all
  PASS under a private dbus-daemon + private `XDG_CONFIG_HOME`,
  timeout-bounded and self-reversing, XTest-state pre-flight: overlay
  select→accept (drag HUD `400x266+355+177` exact logical,
  `last_region` persisted 356,178,400,267, daemon-owned clipboard IHDR
  900x600 outliving the child); editor rect tool with digit sizing
  (stroke pixels exact config `#FF0000`, zero pointer→render offset);
  transparency decision KEEP `with_transparent(true)` (backdrop
  mean=21623/65535, decisively not black); Esc→exit 3 one-shot; color
  picker (`#090D12` clipboard-exact); the three X11 headless reroutes
  (`capture last` → 450x338 PNG with ZERO overlay windows,
  `--region at-cursor` → 2880x1620 with both legs exit 0, offset-less
  `--region 500x300` → 1125x675 cursor-centered exactly); pin lifecycle
  (1472x842 @704,412 exact; drag +145,+95 exact; zoom click4 ⇒ 1560x891
  = 1.03² with Position UNCHANGED proving the TopLeft anchor; opacity
  RMSE(D,F)=0 exact restore); multi-pin with independent close (the
  synthetic-key chain-close fix proven by bisect log); `pins_alive`
  persistence past the idle grace; settings, consent and launcher windows
  on X11 (launcher 900x396 @990,635 exact, X11-probe outputs listed,
  typed-geometry submit → 900x675, cancel → exit 3 in three clean runs);
  riders: clipboard hold-release on SelectionClear (verbatim
  `ownership lost` → `releasing the clipboard-offer hold` → `IdleExit`
  chain, 280 µs) and the stale-daemon self-check on both axes
  (deleted-suffix and identity-mismatch; retry chain exit 0 in ~2.4 s,
  io-failed=0). The run produced four code fixes: pin
  `request_inner_size` on X11, `ResizeAnchor` session routing, the pin
  session-gate exemption, and the synthetic-key filter. One
  non-reproducing transient (exploratory launcher exit=0, clean in three
  re-runs) and one QA-environment defect (stuck XTest Escape) are
  recorded as such in the bundle.

## The deferred GUI-QA batch

Visible-window QA (overlays, pins, dialogs, focus behavior) disturbs whoever
is using the machine, so while the user is present those checks are queued
instead of run. The queue is `.omo/evidence/gui-qa-batch.md`: append-only,
one section per change, each item carrying its verbatim acceptance criterion
and its repro recipe. Items are executed in one consolidated batch when the
user explicitly allows visible windows, and they are tracked as user-gated,
not as failures. A change may complete with GUI acceptance queued; a change
whose GUI portion is the whole point stops and asks instead of skipping
silently.

Before every batch, the QA-environment preflight runs: Hyprland's
`ecosystem:enforce_permissions` must be false (otherwise capture QA stalls
on permission dialogs), `WAYLAND_DISPLAY` and
`HYPRLAND_INSTANCE_SIGNATURE` must be set, and the oracle/injection tools
(grim, wtype, ydotool) must be present.

Currently queued (examples, not exhaustive; the batch manifest is the full
record): live KDE Plasma checks for the ScreenShot2 backend (hardware-gated:
restricted-interface rejection shape, fractional-scale delivery, interactive
picker), the live permission-denial run under Hyprland's
`ecosystem:enforce_permissions` (needs a compositor restart, so it waits for
an explicit maintenance window), and the continuous-motion review of the
animation pass (smoothness at 60 Hz is a human verdict by design). Each is
recorded with its acceptance text in the batch manifest.
