# Architecture decision records

The decisions that shape FlowShot, in ADR form. Each record names the
evidence it stands on; the codebase and the QA evidence files are the ground
truth.

- [ADR-001: Physical-first geometry](adr-001-physical-first-geometry.md): all
  geometry is physical pixels first; scale converts at exactly one boundary.
  The Flameshot #4871 class of mixed-DPI corruption is unrepresentable.
- [ADR-002: Capture backend ladder](adr-002-capture-backend-ladder.md): six
  rungs (five Wayland, one X11), probed at runtime, never table-driven.
- [ADR-003: UI stack](adr-003-ui-stack.md): winit + wgpu + cosmic-text, with
  one recorded exception (egui for the settings window).
- [ADR-004: Daemon lifecycle](adr-004-daemon-lifecycle.md): a single-instance
  session-bus daemon that persists only while something needs it.
- [ADR-005: Secure pixelate](adr-005-secure-pixelate.md): fringe sampling,
  zero interior reads, no insecure mode.
- [ADR-006: Cross-platform gates](adr-006-cross-platform-gates.md): the
  purity contract that keeps Wayland in one crate, and the porting roadmap.
