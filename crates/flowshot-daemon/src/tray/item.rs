//! The `org.kde.StatusNotifierItem` object (host-facing tray surface).
//!
//! Host-compatibility decisions (ksni-verified behaviors):
//! - `ItemIsMenu = false`: LMB (`Activate`) opens the interactive capture
//!   overlay; RMB (`ContextMenu`) opens the `D-Bus` menu. GNOME appindicator
//!   and Plasma < 6.4 call `Activate` first - with this model, that's
//!   "instant capture" which matches the user's desired behavior.
//! - `Status` idles at `Active` (hosts may HIDE `Passive` items).
//! - `IconName` stays empty and the pixmaps carry the icon: no `FlowShot`
//!   themed icon is installed until packaging lands, and an
//!   unresolvable name renders broken on some hosts. The recorded install
//!   name is `org.flowoss.FlowShot` (packaging/ICONS.md).

use zbus::fdo;
use zbus::zvariant::ObjectPath;

use super::TrayCore;
use super::menu::TAKE_SCREENSHOT_ID;
use super::spec::{CATEGORY_APPLICATION, IconWire, MENU_PATH, ToolTipWire};
use crate::strings;

/// The SNI object registered at [`super::spec::ITEM_PATH`].
#[derive(Debug)]
pub(super) struct StatusNotifierItem {
    core: std::sync::Arc<TrayCore>,
}

impl StatusNotifierItem {
    /// An item backed by `core`.
    #[must_use]
    pub(super) const fn new(core: std::sync::Arc<TrayCore>) -> Self {
        Self { core }
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    /// RMB: the host renders the D-Bus menu (the item itself draws nothing).
    #[expect(
        clippy::unused_self,
        reason = "the no-op is protocol policy; the D-Bus menu is served separately"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn context_menu(&self, x: i32, y: i32) -> fdo::Result<()> {
        tracing::trace!(x, y, "tray ContextMenu call (host renders D-Bus menu)");
        Ok(())
    }

    /// LMB: open the interactive capture overlay (same as `flowshot capture`).
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn activate(&self, x: i32, y: i32) -> fdo::Result<()> {
        tracing::trace!(x, y, "tray Activate: opening capture overlay");
        self.core.dispatch(super::menu::TAKE_SCREENSHOT_ID);
        Ok(())
    }

    /// Middle-click: the quick region capture.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn secondary_activate(&self, x: i32, y: i32) -> fdo::Result<()> {
        tracing::trace!(x, y, "tray SecondaryActivate");
        self.core.dispatch(TAKE_SCREENSHOT_ID);
        Ok(())
    }

    /// Scroll events carry no bound action in v1.
    #[expect(
        clippy::unused_self,
        reason = "the no-op is protocol policy, independent of item state"
    )]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn scroll(&self, delta: i32, orientation: String) -> fdo::Result<()> {
        tracing::trace!(delta, %orientation, "tray Scroll (no bound action in v1)");
        Ok(())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn category(&self) -> fdo::Result<String> {
        Ok(CATEGORY_APPLICATION.to_owned())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn id(&self) -> fdo::Result<String> {
        Ok(strings::TRAY_ID.to_owned())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn title(&self) -> fdo::Result<String> {
        Ok(strings::TRAY_TITLE.to_owned())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn status(&self) -> fdo::Result<String> {
        Ok(self.core.status().as_str().to_owned())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn window_id(&self) -> fdo::Result<i32> {
        Ok(0)
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn icon_theme_path(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn icon_name(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn icon_pixmap(&self) -> fdo::Result<Vec<IconWire>> {
        Ok(self.core.icons().idle.clone())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn overlay_icon_name(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> fdo::Result<Vec<IconWire>> {
        Ok(Vec::new())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn attention_icon_name(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> fdo::Result<Vec<IconWire>> {
        Ok(self.core.icons().attention.clone())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn attention_movie_name(&self) -> fdo::Result<String> {
        Ok(String::new())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn tool_tip(&self) -> fdo::Result<ToolTipWire> {
        Ok(ToolTipWire {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: strings::TRAY_TITLE.to_owned(),
            description: "Left-click to capture, right-click for menu".to_owned(),
        })
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[zbus(property)]
    fn menu(&self) -> fdo::Result<ObjectPath<'static>> {
        ObjectPath::try_from(MENU_PATH).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    #[zbus(property)]
    fn item_is_menu(&self) -> fdo::Result<bool> {
        Ok(false)
    }
}
