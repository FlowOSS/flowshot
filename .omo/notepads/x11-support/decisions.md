# Decisions — x11-support

## 2026-10-04 Owner directive: X11 in scope

"i never stated it's ONLY for wayland" — ADR-006 "no X11 code path in v1" non-goal
AMENDED. X11 targets remaining X11-only sessions. Phase A = headless capture path;
Phase B (overlay/editor/pins on X11) deferred to a later plan.

## 2026-10-04 Plan-level decisions (locked in .omo/plans/x11-support.md)

1. New sibling crate flowshot-capture-x11 (purity-gate-exempt platform crate).
2. X11 appended LAST in NEGOTIATION_LADDER (sessions mutually exclusive; preserves Ord invariant).
3. SHM via fd-passing (memfd + attach_fd, SHM>=1.15), GetImage fallback. NO SysV shmat,
   zero new unsafe — allow-list stays at one entry.
4. Scale: Xft.dpi (RESOURCE_MANAGER) preferred, RANDR mm heuristic fallback,
   round to 0.25, clamp [1.0, 4.0], documented approximate. ADR-001 unchanged.
5. Cursor: XQueryPointer one-shot + XFixes image; cursor_events() = None in Phase A.
6. Clipboard: X11Clipboard behind existing ClipboardBackend trait; INCR required;
   Clipboard::for_session() probes session; 6 call sites switch.
7. Daemon session routing: WAYLAND_DISPLAY → wayland probe; else DISPLAY → X11 probe;
   else typed error.
8. UI gate logic unchanged; error message reworded for honesty.
9. request_permission → NotRequired (no X11 permission model).
