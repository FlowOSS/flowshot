//! `org.kde.StatusNotifierItem` wire vocabulary (the freedesktop SNI spec
//! plus the `com.canonical.dbusmenu` menu protocol).
//!
//! Hand-rolled on the daemon's existing zbus-4 connection: `ksni` was
//! cited, but no SNI crate is in the workspace table or the lock and the
//! root manifest is orchestrator-owned - so the protocol surface lives
//! here (ksni's own wire shapes are the reference).

use serde::Serialize;
use zbus::zvariant::{Structure, Type, Value};

/// The watcher's well-known bus name (implemented by the tray host -
/// waybar's tray module owns it on the QA machine).
pub const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";

/// The watcher's interface name (same string, distinct role: zbus's
/// object server rejects member calls that omit the INTERFACE field).
pub const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";

/// The watcher's fixed object path.
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// The item interface name.
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

/// The item's object path. FIXED by the spec/hosts: waybar's watcher maps
/// a registered bus name to `/StatusNotifierItem` (host.cpp
/// `getBusNameAndObjectPath`), so the item MUST live at this path on the
/// daemon's connection.
pub const ITEM_PATH: &str = "/StatusNotifierItem";

/// The menu interface name (`com.canonical.dbusmenu` protocol).
pub const MENU_INTERFACE: &str = "com.canonical.dbusmenu";

/// The menu's object path (carried to hosts through the item's `Menu`
/// property, so any valid path works; `/MenuBar` is the ksni convention).
pub const MENU_PATH: &str = "/MenuBar";

/// The `Category` property value for a normal application tray item.
pub const CATEGORY_APPLICATION: &str = "ApplicationStatus";

/// One `IconPixmap` entry: `(width, height, ARGB32 big-endian bytes)`.
/// The spec's pixel order is A,R,G,B per pixel (most significant byte of
/// the ARGB word first), row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq, Type, Serialize)]
pub struct IconWire {
    /// Pixel width.
    pub width: i32,
    /// Pixel height.
    pub height: i32,
    /// ARGB32 big-endian pixel data (`width * height * 4` bytes).
    pub data: Vec<u8>,
}

/// The `ToolTip` property: `(icon_name, icon_pixmap, title, description)`.
#[derive(Debug, Clone, PartialEq, Eq, Type, Serialize)]
pub struct ToolTipWire {
    /// Themed icon name (empty: the pixmap carries the tooltip icon).
    pub icon_name: String,
    /// Pixmap fallback list.
    pub icon_pixmap: Vec<IconWire>,
    /// Tooltip title.
    pub title: String,
    /// Tooltip subtitle/description.
    pub description: String,
}

// zbus-4's property macro answers `Get` with `Value::from(returned)`, so
// the spec structs teach `Value` their `(iiay)` / `(sa(iiay)ss)` shapes
// (the Type derives already pin the same signatures).
impl From<IconWire> for Value<'_> {
    fn from(icon: IconWire) -> Self {
        Self::Structure(Structure::from((icon.width, icon.height, icon.data)))
    }
}

impl From<ToolTipWire> for Value<'_> {
    fn from(tip: ToolTipWire) -> Self {
        Self::Structure(Structure::from((
            tip.icon_name,
            tip.icon_pixmap,
            tip.title,
            tip.description,
        )))
    }
}

/// The `Status` property vocabulary (spec: `Passive` items may be hidden
/// by hosts, so the daemon idles at `Active`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    /// Normal visible state.
    #[default]
    Active,
    /// Hosts may play `AttentionMovieName` / swap in the attention icon.
    NeedsAttention,
}

impl TrayStatus {
    /// The wire string of this status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_wire_strings_match_the_spec_vocabulary() {
        assert_eq!(TrayStatus::Active.as_str(), "Active");
        assert_eq!(TrayStatus::NeedsAttention.as_str(), "NeedsAttention");
        assert_eq!(TrayStatus::default(), TrayStatus::Active);
    }

    #[test]
    fn wire_signatures_match_the_spec() {
        assert_eq!(IconWire::signature().as_str(), "(iiay)");
        assert_eq!(ToolTipWire::signature().as_str(), "(sa(iiay)ss)");
    }
}
