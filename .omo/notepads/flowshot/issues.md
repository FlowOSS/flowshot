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
