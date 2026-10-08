//! The layered cursor-position strategy and the per-desktop
//! capability table.
//!
//! # The ladder
//!
//! [`resolve_cursor_pos`] walks three layers, strongest first, and traces
//! which layer answered:
//!
//! 1. **ICC cursor session** - the one-shot `ext-image-copy-capture-v1`
//!    pointer-cursor query ([`IccBackend::cursor_pos`], live-verified exact
//!    against `hyprctl cursorpos`), attempted only when the caller passes
//!    the backend, i.e. the negotiated capture backend is ICC.
//! 2. **Hyprland IPC** - the raw v1 socket `cursorpos` query
//!    (`hyprland_ipc`), self-gated on the Hyprland session environment
//!    (`XDG_RUNTIME_DIR` + `HYPRLAND_INSTANCE_SIGNATURE`). No external
//!    process is ever spawned - `hyprctl` is not called (the no-shell-out
//!    rule).
//! 3. **Overlay first motion** - [`CursorSource::AwaitFirstMotion`]: the
//!    position arrives with the first `wl_pointer.motion` after the overlay
//!    maps. Universal: every Wayland compositor delivers
//!    pointer motion to a mapped fullscreen window, which covers KDE
//!    without a `KWin` script and GNOME (no public cursor-position API at
//!    all).
//!
//! Every layer degrades to the next with a log entry; the ladder itself
//! never fails and never panics - `AwaitFirstMotion` catches all desktops.
//!
//! # `KWin`-script bridge for KDE (roadmap, NOT v1)
//!
//! `workspace.cursorPos` via a `KWin` script plus a `D-Bus` bridge (the
//! `plasma-cursor-eyes` precedent) would give KDE a pre-map
//! position like Hyprland's IPC socket. Deliberately not v1: layer 3
//! already covers KDE once the overlay maps, and the bridge would need a
//! script installation with its own failure modes.
//!
//! # Capability table
//!
//! [`cursor_capabilities`] documents which layers answer on which desktop
//! (consumed by the docs build). The runtime ladder is
//! probe/environment-driven, never table-driven: a compositor that grows
//! ICC support is picked up without a release.

use flowshot_capture::DesktopEnv;

use crate::cursor::run_on_worker;
use crate::hyprland_ipc::HyprlandIpc;
use crate::icc::IccBackend;

/// Which layer of the cursor-position ladder answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorSource {
    /// Layer 1: the `ext-image-copy-capture-v1` pointer-cursor session
    /// one-shot. The position is global logical, matching `hyprctl
    /// cursorpos` exactly on Hyprland (live-verified).
    IccCursorSession {
        /// The resolved global logical position.
        position: (i32, i32),
    },
    /// Layer 2: the Hyprland IPC socket `cursorpos` request (global logical,
    /// the value `hyprctl cursorpos` prints).
    HyprlandIpc {
        /// The resolved global logical position.
        position: (i32, i32),
    },
    /// Layer 3: no position is available before the overlay maps. The
    /// consumer defers cursor-preselect until the first
    /// `wl_pointer.motion` arrives - universal across desktops.
    AwaitFirstMotion,
}

impl CursorSource {
    /// The resolved global logical position, or `None` for
    /// [`CursorSource::AwaitFirstMotion`].
    #[must_use]
    pub fn position(&self) -> Option<(i32, i32)> {
        match *self {
            Self::IccCursorSession { position } | Self::HyprlandIpc { position } => Some(position),
            Self::AwaitFirstMotion => None,
        }
    }

    /// The stable layer name used in tracing logs and diagnostics.
    #[must_use]
    pub fn layer_name(&self) -> &'static str {
        match self {
            Self::IccCursorSession { .. } => "icc-cursor-session",
            Self::HyprlandIpc { .. } => "hyprland-ipc",
            Self::AwaitFirstMotion => "overlay-first-motion",
        }
    }
}

/// Resolves the cursor position by walking the layered ladder (module
/// docs).
///
/// `icc` is the negotiated capture backend when it is
/// `ext-image-copy-capture-v1` ([`IccBackend`]), which enables layer 1;
/// `None` masks it. Layer 2 self-gates on the Hyprland session environment.
/// The call never fails: when no layer answers, the result is
/// [`CursorSource::AwaitFirstMotion`] and the position projection
/// ([`CursorSource::position`]) is `None`.
///
/// # Examples
///
/// ```no_run
/// # futures::executor::block_on(async {
/// use flowshot_capture_wayland::{IccBackend, resolve_cursor_pos};
///
/// let source = resolve_cursor_pos(Some(IccBackend::new())).await;
/// if let Some((x, y)) = source.position() {
///     println!("cursor at global logical ({x}, {y}) via {}", source.layer_name());
/// }
/// # });
/// ```
pub async fn resolve_cursor_pos(icc: Option<IccBackend>) -> CursorSource {
    resolve_with(icc, HyprlandIpc::from_env()).await
}

/// The ladder itself with both optional layers injected - the test seam
/// that replaces the environment-derived layer-2 target with a fixture.
async fn resolve_with(icc: Option<IccBackend>, hyprland: Option<HyprlandIpc>) -> CursorSource {
    if let Some(backend) = icc {
        if let Some(position) = backend.cursor_pos().await {
            tracing::info!(
                layer = "icc-cursor-session",
                x = position.0,
                y = position.1,
                "cursor position resolved"
            );
            return CursorSource::IccCursorSession { position };
        }
        tracing::debug!(
            layer = "icc-cursor-session",
            "cursor layer 1 reported no position; falling through"
        );
    } else {
        tracing::debug!(
            layer = "icc-cursor-session",
            "cursor layer 1 masked: the negotiated backend is not ext-image-copy-capture"
        );
    }
    if let Some(ipc) = hyprland {
        let position = run_on_worker("flowshot-hyprland-ipc", move || match ipc.cursor_pos() {
            Ok(position) => Some(position),
            Err(error) => {
                tracing::warn!(%error, "cursor layer 2 (Hyprland IPC) failed; falling through");
                None
            }
        })
        .await;
        if let Some(position) = position {
            tracing::info!(
                layer = "hyprland-ipc",
                x = position.0,
                y = position.1,
                "cursor position resolved"
            );
            return CursorSource::HyprlandIpc { position };
        }
    } else {
        tracing::debug!(
            layer = "hyprland-ipc",
            "cursor layer 2 masked: XDG_RUNTIME_DIR or HYPRLAND_INSTANCE_SIGNATURE unset"
        );
    }
    tracing::info!(
        layer = "overlay-first-motion",
        "cursor position unresolved; deferring to the overlay's first pointer motion"
    );
    CursorSource::AwaitFirstMotion
}

/// Which cursor-position layers a desktop environment can answer.
///
/// Layer 3 (overlay first motion) is deliberately not a flag: it is
/// universal - every Wayland compositor delivers `wl_pointer.motion` to a
/// mapped fullscreen overlay, so it applies on every desktop,
/// including KDE without a `KWin` script and GNOME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorCapabilities {
    /// Layer 1: an `ext-image-copy-capture-v1` pointer-cursor session is
    /// expected to answer. Per-desktop evidence lives on
    /// [`cursor_capabilities`]; the runtime probe is always authoritative.
    pub icc_cursor_session: bool,
    /// Layer 2: the Hyprland IPC socket is expected to answer (Hyprland
    /// only, by definition of the socket).
    pub hyprland_ipc: bool,
}

/// Every desktop the capability table covers, for docs generation.
pub const CURSOR_CAPABILITY_DESKTOPS: [DesktopEnv; 7] = [
    DesktopEnv::Hyprland,
    DesktopEnv::Sway,
    DesktopEnv::Niri,
    DesktopEnv::Cosmic,
    DesktopEnv::Kde,
    DesktopEnv::Gnome,
    DesktopEnv::Other,
];

/// The documented cursor-position capabilities of `desktop`.
///
/// Evidence per row (the runtime ladder never consults this table - the
/// live probe and environment decide):
///
/// - `Hyprland`: ICC live-verified (exact `hyprctl cursorpos`
///   match); IPC live-verified (v1 socket, JSON `cursorpos` reply).
/// - `Sway`: ICC from wlroots 0.19+ / sway 1.10+ (cursor-session behavior
///   source-verified against wlroots; `wayland.app` lists Sway 1.11 with
///   `ext_image_copy_capture_manager_v1`). Not live-verified here.
/// - `Cosmic`: ICC from cosmic-comp's `ext-image-copy-capture-v1` handler
///   (`new_cursor_session` + `set_cursor_pos` source-verified;
///   `wayland.app` lists COSMIC beta 8 with the manager). Not
///   live-verified here.
/// - `Niri`: no ICC (`wayland.app` lists niri 26.04 without the manager;
///   upstream PR #3942 is open and implements no cursor session).
/// - `Kde`: no ICC (`KWin` 6.7 does not implement it; capture there is the
///   `ScreenShot2` `D-Bus` path). The `KWin`-script cursor bridge
///   is a roadmap enhancement (module docs).
/// - `Gnome`: no ICC (mutter does not implement it, and GNOME exposes no
///   public cursor-position API at all).
/// - `Other`: nothing assumed; layers 1-2 self-gate on the live probe and
///   environment, layer 3 always applies.
#[must_use]
pub const fn cursor_capabilities(desktop: DesktopEnv) -> CursorCapabilities {
    match desktop {
        DesktopEnv::Hyprland => CursorCapabilities {
            icc_cursor_session: true,
            hyprland_ipc: true,
        },
        DesktopEnv::Sway | DesktopEnv::Cosmic => CursorCapabilities {
            icc_cursor_session: true,
            hyprland_ipc: false,
        },
        DesktopEnv::Niri | DesktopEnv::Kde | DesktopEnv::Gnome | DesktopEnv::Other => {
            CursorCapabilities {
                icc_cursor_session: false,
                hyprland_ipc: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use super::*;
    use crate::hyprland_ipc::test_support::spawn_fake_socket;

    /// The exact reply bytes observed live on Hyprland 0.56.2.
    const LIVE_REPLY: &str = "\n{\n    \"x\": 3200,\n    \"y\": 720\n}\n";

    fn resolve_with_fake_socket(reply: Option<&'static str>, tag: &str) -> CursorSource {
        let (ipc, dir, server) = spawn_fake_socket(tag, reply, Duration::from_millis(500));
        let source = futures::executor::block_on(resolve_with(None, Some(ipc)));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        source
    }

    #[test]
    fn all_layers_masked_degrades_to_await_first_motion() {
        // Given no ICC backend and no Hyprland session environment
        // When the ladder runs
        // Then it hands off to the overlay cleanly - no position, no panic.
        let source = futures::executor::block_on(resolve_with(None, None));
        assert_eq!(source, CursorSource::AwaitFirstMotion);
        assert_eq!(source.position(), None);
        assert_eq!(source.layer_name(), "overlay-first-motion");
    }

    #[test]
    fn hyprland_ipc_answers_when_layer_one_is_masked() {
        let source = resolve_with_fake_socket(Some(LIVE_REPLY), "resolve-happy");
        assert_eq!(
            source,
            CursorSource::HyprlandIpc {
                position: (3200, 720)
            }
        );
        assert_eq!(source.position(), Some((3200, 720)));
        assert_eq!(source.layer_name(), "hyprland-ipc");
    }

    #[test]
    fn failing_hyprland_ipc_falls_through_to_first_motion() {
        let source = resolve_with_fake_socket(Some("unknown request"), "resolve-garbage");
        assert_eq!(source, CursorSource::AwaitFirstMotion);
    }

    #[test]
    fn position_and_layer_name_map_every_variant() {
        let icc = CursorSource::IccCursorSession { position: (1, 2) };
        assert_eq!(icc.position(), Some((1, 2)));
        assert_eq!(icc.layer_name(), "icc-cursor-session");
    }

    #[test]
    fn capability_table_matches_the_f13_verdicts() {
        assert_eq!(
            cursor_capabilities(DesktopEnv::Hyprland),
            CursorCapabilities {
                icc_cursor_session: true,
                hyprland_ipc: true
            }
        );
        for desktop in [DesktopEnv::Sway, DesktopEnv::Cosmic] {
            assert_eq!(
                cursor_capabilities(desktop),
                CursorCapabilities {
                    icc_cursor_session: true,
                    hyprland_ipc: false
                },
                "{desktop:?}"
            );
        }
        for desktop in [
            DesktopEnv::Niri,
            DesktopEnv::Kde,
            DesktopEnv::Gnome,
            DesktopEnv::Other,
        ] {
            assert_eq!(
                cursor_capabilities(desktop),
                CursorCapabilities {
                    icc_cursor_session: false,
                    hyprland_ipc: false
                },
                "{desktop:?}"
            );
        }
    }

    #[test]
    fn only_hyprland_gets_the_ipc_layer() {
        for desktop in CURSOR_CAPABILITY_DESKTOPS {
            assert_eq!(
                cursor_capabilities(desktop).hyprland_ipc,
                desktop == DesktopEnv::Hyprland,
                "{desktop:?}"
            );
        }
    }
}
