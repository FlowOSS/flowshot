//! The `org.kde.StatusNotifierItem` object (host-facing tray surface).
//!
//! Host-compatibility decisions (ksni-verified behaviors):
//! - `ItemIsMenu = true` and `Activate` answers `UnknownMethod`: the GNOME
//!   appindicator extension and Plasma < 6.4 call `Activate` first and only
//!   fall back to the `D-Bus` menu on that error; waybar renders the menu
//!   itself and never calls `Activate`.
//! - `Status` idles at `Active` (hosts may HIDE `Passive` items).
//! - `IconName` stays empty and the pixmaps carry the icon: no `FlowShot`
//!   themed icon is installed until packaging lands (todo 41/42), and an
//!   unresolvable name renders broken on some hosts.

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
    /// The host asks the item to draw its own menu; hosts that reach this
    /// despite `ItemIsMenu` render the `D-Bus` menu instead, so this is a
    /// successful no-op.
    #[expect(
        clippy::unused_self,
        reason = "the no-op is protocol policy, independent of item state"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn context_menu(&self, x: i32, y: i32) -> fdo::Result<()> {
        tracing::trace!(
            x,
            y,
            "tray ContextMenu call (the host renders the D-Bus menu)"
        );
        Ok(())
    }

    /// Deliberate `UnknownMethod` while `ItemIsMenu` is true (see the
    /// module docs - GNOME/Plasma menu fallback).
    #[expect(
        clippy::unused_self,
        reason = "the rejection is protocol policy, independent of item state"
    )]
    fn activate(&self, x: i32, y: i32) -> fdo::Result<()> {
        tracing::trace!(x, y, "tray Activate rejected: the item is menu-driven");
        Err(fdo::Error::UnknownMethod(
            "the item is menu-driven (ItemIsMenu)".to_owned(),
        ))
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
            description: String::new(),
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
        Ok(true)
    }
}
