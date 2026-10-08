//! Launch-time overlay flows: preselect-at-cursor, region
//! memory, accept-on-select.
//!
//! This module is the UI-side launch boundary the binary layer
//! maps the CLI's typed invocation onto - NO CLI parsing lives here (a
//! MUST-NOT; the `--region` grammar is the CLI's). The mapping
//! contract, from `flowshot-cli`'s `invocation.rs` vocabulary:
//!
//! | CLI / config source | UI boundary |
//! |---|---|
//! | `RegionToken::Rect { w, h, Some(x), Some(y) }` | [`Preselect::Region`] with `origin: Some` (explicit coords) |
//! | `RegionToken::Rect { w, h, None, None }` | [`Preselect::Region`] with `origin: None` (centered at cursor) |
//! | `RegionToken::AtCursor` | [`Preselect::OutputAtCursor`] (the output under the cursor becomes the selection - the interactive form of the output-at-cursor behavior; recorded decision, the binary layer's launch flows confirm) |
//! | `request.last_region` + `[capture].last_region` | [`Preselect::LastRegion`] (the binary layer reads the TOML) |
//! | `request.instant` | [`LaunchRequest::instant`] |
//! | `[capture].save_last_region` | [`LaunchRequest::save_last_region`] |
//! | the cursor probe's `resolve_cursor_pos().position()` | [`LaunchRequest::cursor`] (`None` = `AwaitFirstMotion`) |
//! | `ScreenSpec::Cursor` (`capture screen`, no arg) | [`output_at_cursor`] - the output-at-cursor resolution (a recorded decision), NOT an overlay flow |
//!
//! `request.delay_ms`/`no_edit`/action flags stay binary-layer concerns
//! (capture timing and export wiring in the binary layer); they seed no overlay
//! state.
//!
//! # Coordinate space
//!
//! `--region` coordinates are GLOBAL LOGICAL pixels - the selection
//! engine's space, where one rect spans every monitor. The
//! physical-first rule applies at export: each output's crop is computed
//! with that output's OWN scale via
//! [`OutputLayout::crop_rects`](flowshot_core::geometry::OutputLayout::crop_rects)
//! (never an averaged factor), so a preselected logical rect on a scale-2
//! output exports at exactly twice the physical size (pinned in tests).
//!
//! # `AwaitFirstMotion` (the cursor-resolution contract)
//!
//! When the cursor is unresolved ([`LaunchRequest::cursor`] is `None`), a
//! cursor-dependent preselect (centered region, output-at-cursor) defers to
//! a [`PendingPreselect`] that the route funnel applies on the FIRST pointer
//! motion the overlay receives - the universal layer-3 fallback. Explicit
//! coordinates and a persisted last region need no cursor and seed
//! immediately.
//!
//! # Region memory
//!
//! With [`LaunchRequest::save_last_region`], every capture-completing
//! gesture (Enter/instant-release [`Accept`](crate::input::Action::Accept),
//! or a [`Copy`](crate::input::Action::Copy)) writes the selection through
//! the [`RegionSink`] callback - the binary layer owns the TOML path
//! (`[capture].last_region`; the `DrawColorSink` precedent keeps this crate
//! pure).

mod resolve;

#[cfg(test)]
mod tests;

use flowshot_core::geometry::LogicalPoint;

use crate::input::Action;
use crate::router::WindowSlot;
use crate::selection::format_geometry;
use crate::state::OverlayCore;

pub use resolve::{InitialSelection, PendingPreselect, Preselect, output_at_cursor};

/// The `[capture].last_region` write-back callback (region memory; the
/// binary layer owns the TOML path - the `DrawColorSink` precedent).
pub type RegionSink = Box<dyn Fn(flowshot_core::config::Region) + Send + 'static>;

/// One typed overlay-launch request (the binary-layer boundary; see the
/// module header for the CLI mapping table).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LaunchRequest {
    /// What the selection engine seeds with (the preselect vocabulary).
    pub preselect: Preselect,
    /// The resolved global logical cursor position (the cursor probe's
    /// `resolve_cursor_pos`); `None` = `AwaitFirstMotion` (defer a
    /// cursor-dependent preselect to the first overlay motion event).
    pub cursor: Option<LogicalPoint>,
    /// `--instant`: the first left release that leaves a selection emits
    /// [`Action::Accept`] immediately - accept-on-select, no editor review
    /// step (fixes flameshot#4780's monitor-picker blocking: `FlowShot` has no
    /// picker; the spanning overlay handles multi-monitor natively).
    pub instant: bool,
    /// `[capture].save_last_region`: persist the selection through the
    /// [`RegionSink`] on every capture-completing gesture.
    ///
    /// `Default` is `false` (a bare launch persists nothing); the binary
    /// layer projects the config key (whose own default is `true`).
    pub save_last_region: bool,
}

/// The core-owned launch state: the stored request awaiting the layout, the
/// deferred preselect, the instant arming, and the region-memory sink.
#[derive(Default)]
pub(crate) struct LaunchState {
    request: Option<LaunchRequest>,
    seeded: bool,
    pending: Option<PendingPreselect>,
    instant_armed: bool,
    save_last_region: bool,
    sink: Option<RegionSink>,
}

impl std::fmt::Debug for LaunchState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchState")
            .field("request", &self.request)
            .field("seeded", &self.seeded)
            .field("pending", &self.pending)
            .field("instant_armed", &self.instant_armed)
            .field("save_last_region", &self.save_last_region)
            .field("sink", &self.sink.is_some())
            .finish()
    }
}

impl OverlayCore {
    /// Seeds the overlay from a typed launch request.
    ///
    /// Callable before the event loop runs: the router layout arrives on
    /// window spawn, so the preselect resolution runs at
    /// [`Self::launch`]-time when a layout is already installed (headless
    /// cores, the QA harness) and otherwise at the spawn hook. A
    /// cursor-dependent preselect with an unresolved cursor defers to the
    /// first motion event (the `AwaitFirstMotion` contract). A later
    /// call supersedes an earlier request (last writer wins).
    pub fn launch(&mut self, request: LaunchRequest) {
        tracing::info!(
            target: "flowshot_ui::launch",
            preselect = request.preselect.token(),
            cursor = ?request.cursor.map(|point| (point.x.0, point.y.0)),
            instant = request.instant,
            save_last_region = request.save_last_region,
            "launch request"
        );
        self.launch.instant_armed = request.instant;
        self.launch.save_last_region = request.save_last_region;
        self.launch.seeded = false;
        self.launch.pending = None;
        self.launch.request = Some(request);
        self.apply_launch();
    }

    /// Installs the region-memory persistence sink (`None` clears; the
    /// `[capture].last_region` write-back seam - the binary layer owns the
    /// TOML path, the `DrawColorSink` precedent).
    pub fn set_region_sink(&mut self, sink: Option<RegionSink>) {
        self.launch.sink = sink;
    }

    /// Resolves the stored launch request against the installed layout
    /// (the spawn hook: the router layout arrives on `Resumed`, after the
    /// binary layer called [`Self::launch`]). A no-op when no request is
    /// stored, the request is already seeded, or the layout is still empty.
    pub(crate) fn apply_launch(&mut self) {
        if self.launch.seeded {
            return;
        }
        let Some(request) = self.launch.request else {
            return;
        };
        if self.router.layout().outputs.is_empty() {
            return;
        }
        self.launch.seeded = true;
        let initial = request
            .preselect
            .resolve(request.cursor, self.router.layout());
        match initial {
            InitialSelection::Ready(rect) => {
                if let Some(rect) = rect {
                    tracing::info!(
                        target: "flowshot_ui::launch",
                        geometry = %format_geometry(rect),
                        "preselect seeded"
                    );
                } else {
                    tracing::info!(target: "flowshot_ui::launch", "launch without preselect");
                }
                self.selection.set_rect(rect);
            }
            InitialSelection::Deferred(pending) => {
                tracing::info!(
                    target: "flowshot_ui::launch",
                    kind = pending.token(),
                    "preselect deferred until first motion"
                );
                self.launch.pending = Some(pending);
            }
        }
    }

    /// The funnel's first-motion hook: applies a deferred preselect
    /// ([`PendingPreselect`]) at the motion's clamped global position and
    /// reports the all-window redraws; empty when nothing is pending. The
    /// pending preselect is consumed whether or not it resolves (the first
    /// motion IS the cursor-resolution point - a position outside every
    /// output degrades to no preselect, warned, never retried per motion).
    pub(crate) fn launch_on_motion(&mut self, at: LogicalPoint) -> Vec<Action> {
        let Some(pending) = self.launch.pending.take() else {
            return Vec::new();
        };
        let rect = pending.apply(at, self.router.layout());
        if let Some(rect) = rect {
            tracing::info!(
                target: "flowshot_ui::launch",
                geometry = %format_geometry(rect),
                kind = pending.token(),
                "deferred preselect applied on first motion"
            );
        } else {
            tracing::warn!(
                target: "flowshot_ui::launch",
                kind = pending.token(),
                "deferred preselect unresolved at first motion; continuing without preselect"
            );
        }
        self.selection.set_rect(rect);
        (0..self.router.window_count())
            .map(|index| Action::Redraw(WindowSlot::new(index)))
            .collect()
    }

    /// The funnel's left-release hook: `--instant` accept-on-select. Fires
    /// [`Action::Accept`] on the first release that leaves a selection
    /// (a dragged rect, a kept preselect, or a moved/resized preselect) and
    /// disarms; a release that clears the selection (a click outside) keeps
    /// the arming for a later drag. Empty when instant mode is off.
    pub(crate) fn launch_on_release(&mut self) -> Vec<Action> {
        if !self.launch.instant_armed {
            return Vec::new();
        }
        let Some(rect) = self.selection.rect() else {
            return Vec::new();
        };
        self.launch.instant_armed = false;
        tracing::info!(
            target: "flowshot_ui::launch",
            geometry = %format_geometry(rect),
            effect = "accept",
            "instant accept on first release"
        );
        vec![Action::Accept]
    }

    /// The funnel's exit hook (region memory): when `actions` carries a
    /// capture-completing effect ([`Action::Accept`], [`Action::Copy`], or
    /// one of the toolbar completions Save/Pin/Upload/OpenWith),
    /// persists the current selection through the [`RegionSink`] (gated on
    /// `[capture].save_last_region`) and disarms the instant accept - any
    /// accept completes the session, so a later release must not re-fire.
    pub(crate) fn launch_persist(&mut self, actions: &[Action]) {
        let captured = actions.iter().any(|action| {
            matches!(
                action,
                Action::Accept
                    | Action::Copy
                    | Action::Save
                    | Action::Pin
                    | Action::Upload
                    | Action::OpenWith
            )
        });
        if !captured {
            return;
        }
        if actions.contains(&Action::Accept) {
            self.launch.instant_armed = false;
        }
        if !self.launch.save_last_region {
            return;
        }
        let Some(rect) = self.selection.rect() else {
            return;
        };
        let Some(region) = resolve::region_of(rect) else {
            tracing::warn!(
                target: "flowshot_ui::launch",
                "selection rect exceeds the persisted region range; last region not saved"
            );
            return;
        };
        let Some(sink) = self.launch.sink.as_ref() else {
            tracing::debug!(
                target: "flowshot_ui::launch",
                "no region sink installed; last region not persisted"
            );
            return;
        };
        tracing::info!(
            target: "flowshot_ui::launch",
            x = region.x,
            y = region.y,
            width = region.width,
            height = region.height,
            "last region persisted"
        );
        sink(region);
    }
}
