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
