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

Live QA runs on the development machine: Hyprland 0.56.2, NVIDIA GPU, two
monitors at scale 1 (plus synthetic headless outputs for scale-2 and rotated
fixtures), and an i3 session on X11 (single eDP-1 2880x1620, derived scale
2.25) for the X11 capture path. Every claim in FlowShot's docs and evidence
carries one of these classes:

- **LIVE-verified**: exercised on the real session, with the observed values
  recorded (pixel oracles via grim, `hyprctl`/`busctl` reads, logs).
- **Source-verified**: the behavior is pinned against the other party's
  source code at a recorded commit (wire contracts, protocol quirks), but
  the path was not run live on that desktop.
- **Unit-level**: covered by tests only; the live path is explicitly not
  claimed.
- **Hardware-gated**: cannot run on the QA machine at all (no KDE or GNOME
  session, no touchscreen). Never claimed, scheduled for suitable hardware.
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

- **X11 Phase A headless capture** (2026-10-04, **LIVE-verified**):
  `.omo/evidence/x11-phase-a/`. Ten checks on the live i3 session, all PASS:
  full capture pixel-cross-checked against an independent
  `import -window root` oracle, `--region WxH+X+Y` geometry pixel-exact
  against a crop of the full frame, screen by connector name and by index,
  `--raw` stdout PNG, `--print-geometry`, daemon-owned CLIPBOARD with INCR
  (a 1.08 MB offer served after the CLI exited and gone after the daemon
  was killed), the exit-code matrix (0/1/2/4), SHM fd-passing vs plain
  timing (38 ms vs 80 ms), the `-d` delay path, and daemon idle-exit
  lifecycle. Per-check values and environment dump: `index.txt` in the
  bundle.

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
