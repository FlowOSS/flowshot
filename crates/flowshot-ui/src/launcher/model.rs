//! The launcher's typed dialog state: the target selection (manual geometry
//! vs one probed monitor), the geometry entry with its validation, and the
//! delay. Pure model - the widget layer ([`super::ui`]) projects it and the
//! [`LauncherModel::request`] seam is the single dispatch decision point.

use flowshot_core::geometry::OutputInfo;

use super::request::{GeometryIssue, LauncherRequest, RegionGeometry};
use super::strings;

/// One monitor dropdown entry (the live probe's output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorEntry {
    /// The output index in probe order - the daemon's `CaptureScreen`
    /// vocabulary (the tray submenu uses the same indexing).
    pub screen: u32,
    /// The dropdown label (`Screen {n}: {name}`, the tray's format).
    pub label: String,
}

/// What the Capture button will dispatch to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Target {
    /// The manual geometry entry (region semantics).
    #[default]
    Manual,
    /// One probed monitor, by its probe-order index.
    Monitor(u32),
}

/// The launcher dialog state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherModel {
    monitors: Vec<MonitorEntry>,
    target: Target,
    geometry_text: String,
    delay_ms: u32,
    focus_armed: bool,
}

impl Default for LauncherModel {
    fn default() -> Self {
        Self {
            monitors: Vec::new(),
            target: Target::default(),
            geometry_text: String::new(),
            delay_ms: 0,
            focus_armed: true,
        }
    }
}

impl LauncherModel {
    /// A model whose monitor dropdown lists `outputs` in probe order (the
    /// `CaptureScreen` index authority). Indices beyond `u32` are dropped
    /// (unreachable: the wire vocabulary itself is `u32`).
    #[must_use]
    pub fn from_outputs(outputs: Vec<OutputInfo>) -> Self {
        let monitors = outputs
            .into_iter()
            .enumerate()
            .filter_map(|(index, output)| {
                u32::try_from(index).ok().map(|screen| MonitorEntry {
                    screen,
                    label: monitor_label(screen, &output),
                })
            })
            .collect();
        Self {
            monitors,
            ..Self::default()
        }
    }

    /// The monitor dropdown entries (probe order).
    #[must_use]
    pub fn monitors(&self) -> &[MonitorEntry] {
        &self.monitors
    }

    /// The current target selection.
    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }

    /// The target selection, for the dropdown binding.
    pub fn target_mut(&mut self) -> &mut Target {
        &mut self.target
    }

    /// The raw geometry entry text.
    #[must_use]
    pub fn geometry_text(&self) -> &str {
        &self.geometry_text
    }

    /// The raw geometry entry text, for the field binding.
    pub fn geometry_text_mut(&mut self) -> &mut String {
        &mut self.geometry_text
    }

    /// The delay spinner value in milliseconds.
    #[must_use]
    pub const fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    /// The delay spinner value, for the widget binding.
    pub fn delay_ms_mut(&mut self) -> &mut u32 {
        &mut self.delay_ms
    }

    /// The geometry entry's validation state (the inline error source).
    ///
    /// # Errors
    ///
    /// [`GeometryIssue::Empty`] while the field is blank (the UI shows the
    /// grammar hint), [`GeometryIssue::Malformed`] for anything outside the
    /// `WxH[+X+Y]` grammar.
    pub fn geometry(&self) -> Result<RegionGeometry, GeometryIssue> {
        RegionGeometry::parse(&self.geometry_text)
    }

    /// The dispatch seam: what Capture would emit right now, or `None` when
    /// the dialog has no valid request (manual target with an invalid
    /// geometry entry - the Capture button is disabled in exactly that
    /// state).
    #[must_use]
    pub fn request(&self) -> Option<LauncherRequest> {
        match self.target {
            Target::Manual => self
                .geometry()
                .ok()
                .map(|geometry| LauncherRequest::Region {
                    geometry,
                    delay_ms: self.delay_ms,
                }),
            Target::Monitor(screen) => Some(LauncherRequest::Screen {
                screen,
                delay_ms: self.delay_ms,
            }),
        }
    }

    /// Consumes the one-shot geometry-field focus arming (the dialog opens
    /// ready to type).
    pub fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus_armed)
    }
}

/// `Screen {n}: {name}` - the live probe's name already carries make, model
/// AND connector (the tray label rule); the connector is the
/// fallback for a nameless output.
fn monitor_label(screen: u32, output: &OutputInfo) -> String {
    let display = if output.name.is_empty() {
        output.connector.as_str()
    } else {
        output.name.as_str()
    };
    format!("{} {screen}: {display}", strings::TARGET_SCREEN_PREFIX)
}
