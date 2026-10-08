# Decisions — x11-phase-b

## 2026-10-04 Plan-level decisions (locked in .omo/plans/x11-phase-b.md)

1. Winit-only overlay first (fullscreen + AlwaysOnTop + auto-focus);
   override-redirect + x11rb grabs = documented contingency only.
2. Transparency not load-bearing (frozen-capture backdrop is opaque);
   transparent(false) on X11 only if live evidence shows black-screen.
3. require_display_server accepts DISPLAY; NoDisplayServer only when neither
   is set; all six interactive paths inherit.
4. One shared session_window_customizer helper in daemon (Wayland/X11 arms).
5. Launcher probe_outputs gains an X11Backend leg.
6. Riders: last/at-cursor reroutes fold into task 2 (same seam); clipboard
   hold-release + stale-daemon self-check in task 4.
7. Live QA via xdotool synthetic input + self-capture evidence.
