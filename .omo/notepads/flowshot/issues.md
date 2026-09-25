# Issues — flowshot

Problems and gotchas encountered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## 2026-09-25: git repo never initialized (USER-CAUGHT DEFECT)
Todo 1 required `git init` (plan Commit strategy: "git init happens in todo 1") + one commit per todo. Worker claimed done without it; orchestrator verification missed it (ran build/test/clippy but never checked `git rev-parse`). Todos 1-4 landed uncommitted.
RECOVERY: git init -b main, reconstructed buildable sequential commits per todo (intermediate lib.rs states match each subagent's verified-green state: skeleton -> +tokens/config -> +geometry -> +scene).
PREVENTION: verification gate now includes `git rev-parse --git-dir` + `git status` after EVERY todo commit step; worker prompts explicitly forbid worker-side git (orchestrator owns commits, avoids index.lock races in parallel waves).
