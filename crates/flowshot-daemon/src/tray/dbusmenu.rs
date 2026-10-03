//! The `com.canonical.dbusmenu` object: the tray menu served to the host
//! (waybar renders it through libdbusmenu; the panel draws, the daemon
//! only answers `GetLayout` / `Event` callbacks).
//!
//! Wire shape: `GetLayout` replies `(u revision, (i id, a{sv} props,
//! av children))` - children are recursively the same structure wrapped
//! in variants. [`Layout`] derives that signature; child structs are
//! built through the infallible tuple `From` impl (`Value::from((..))` =
//! `Value::Structure`; lesson learned: manual `Value`/`Dict` assembly is
//! a rabbit hole, and zvariant 5's `StructureBuilder::build` is fallible).
//!
//! All methods are SYNC (sync dispatch runs inline on the connection's
//! dispatch task - the zbus-5 tokio reactor task when built inside a
//! tokio runtime, the private async-io driver thread otherwise): the
//! layout answers from the cached probe, and the only slow operation -
//! the wayland output probe - is pushed onto the tokio runtime through
//! [`TrayCore::refresh_outputs`].

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use zbus::fdo;
use zbus::zvariant::{OwnedValue, Type, Value};

use super::TrayCore;
use super::menu::{MenuNode, ROOT_ID, SCREEN_SUBMENU_ID, find_node};

/// One `GetLayout` node: `(id, properties, children-as-variants)`.
#[derive(Debug, Type, Serialize)]
struct Layout {
    id: i32,
    properties: HashMap<String, Value<'static>>,
    children: Vec<Value<'static>>,
}

/// The dbusmenu object registered at [`super::spec::MENU_PATH`].
#[derive(Debug)]
pub(super) struct DbusMenu {
    core: Arc<TrayCore>,
}

impl DbusMenu {
    /// A menu backed by `core`.
    #[must_use]
    pub(super) const fn new(core: Arc<TrayCore>) -> Self {
        Self { core }
    }

    fn layout_for(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        filter: &[String],
    ) -> Option<Layout> {
        let menu = self.core.menu_nodes();
        if parent_id == ROOT_ID {
            return Some(Layout {
                id: ROOT_ID,
                properties: HashMap::new(),
                children: children_values(&menu, recursion_depth, filter),
            });
        }
        find_node(&menu, parent_id).map(|node| layout_of(node, recursion_depth, filter))
    }
}

fn layout_of(node: &MenuNode, recursion_depth: i32, filter: &[String]) -> Layout {
    Layout {
        id: node.id,
        properties: node_props(node, filter),
        children: children_values(&node.children, recursion_depth, filter),
    }
}

fn children_values(
    nodes: &[MenuNode],
    recursion_depth: i32,
    filter: &[String],
) -> Vec<Value<'static>> {
    if recursion_depth == 0 {
        return Vec::new();
    }
    let child_depth = if recursion_depth < 0 {
        -1
    } else {
        recursion_depth - 1
    };
    nodes
        .iter()
        .map(|node| {
            Value::from((
                node.id,
                node_props(node, filter),
                children_values(&node.children, child_depth, filter),
            ))
        })
        .collect()
}

fn node_props(node: &MenuNode, filter: &[String]) -> HashMap<String, Value<'static>> {
    let mut props = HashMap::new();
    let mut put = |key: &str, value: Value<'static>| {
        if filter.is_empty() || filter.iter().any(|wanted| wanted == key) {
            props.insert(key.to_owned(), value);
        }
    };
    match &node.label {
        Some(label) => put("label", Value::from(label.clone())),
        None => put("type", Value::from("separator")),
    }
    put("enabled", Value::Bool(node.enabled));
    put("visible", Value::Bool(true));
    if node.submenu {
        put("children-display", Value::from("submenu"));
    }
    props
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    /// The layout below `parent_id` (`recursion_depth` -1 = unlimited,
    /// 0 = the node alone; `property_names` empty = all properties).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        self.layout_for(parent_id, recursion_depth, &property_names)
            .map(|layout| (self.core.revision(), layout))
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown parent id {parent_id}")))
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> fdo::Result<Vec<(i32, HashMap<String, Value<'static>>)>> {
        let menu = self.core.menu_nodes();
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                find_node(&menu, id).map(|node| (id, node_props(node, &property_names)))
            })
            .collect())
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedValue> {
        let menu = self.core.menu_nodes();
        let node = find_node(&menu, id)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown id {id}")))?;
        let value = node_props(node, std::slice::from_ref(&name))
            .remove(&name)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("unknown property {name}")))?;
        value
            .try_to_owned()
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// The activation callback: `clicked` runs the id's dispatch-table
    /// action (the [`CommandSink`](crate::CommandSink) seam).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn event(
        &self,
        id: i32,
        event_id: String,
        data: OwnedValue,
        timestamp: u32,
    ) -> fdo::Result<()> {
        if !self.core.has_id(id) {
            return Err(fdo::Error::InvalidArgs(format!("unknown id {id}")));
        }
        if event_id == "clicked" {
            self.core.dispatch(id);
        } else {
            tracing::debug!(id, event = %event_id, data = ?data, timestamp, "ignoring non-activation menu event");
        }
        Ok(())
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> fdo::Result<Vec<i32>> {
        if events.is_empty() {
            return Err(fdo::Error::InvalidArgs("empty event group".to_owned()));
        }
        let events_len = events.len();
        let mut not_found = Vec::new();
        for (id, event_id, _data, _timestamp) in events {
            if !self.core.has_id(id) {
                not_found.push(id);
            } else if event_id == "clicked" {
                self.core.dispatch(id);
            }
        }
        if not_found.len() == events_len {
            return Err(fdo::Error::InvalidArgs(
                "none of the event ids are known".to_owned(),
            ));
        }
        Ok(not_found)
    }

    /// Opening the per-monitor submenu refreshes the live probe in the
    /// background; the answer stays `false` (ksni/Qt behavior: updates
    /// arrive through the `LayoutUpdated` signal, not the return value).
    fn about_to_show(&self, id: i32) -> fdo::Result<bool> {
        if !self.core.has_id(id) {
            return Err(fdo::Error::InvalidArgs(format!("unknown id {id}")));
        }
        if id == SCREEN_SUBMENU_ID {
            self.core.refresh_outputs();
        }
        Ok(false)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn about_to_show_group(&self, ids: Vec<i32>) -> fdo::Result<(Vec<i32>, Vec<i32>)> {
        let mut not_found = Vec::new();
        for id in ids {
            if id == ROOT_ID {
                continue;
            }
            if !self.core.has_id(id) {
                not_found.push(id);
            } else if id == SCREEN_SUBMENU_ID {
                self.core.refresh_outputs();
            }
        }
        Ok((Vec::new(), not_found))
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[zbus(property)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn version(&self) -> fdo::Result<u32> {
        Ok(3)
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[zbus(property)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn text_direction(&self) -> fdo::Result<String> {
        Ok("ltr".to_owned())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[zbus(property)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn status(&self) -> fdo::Result<String> {
        Ok("normal".to_owned())
    }

    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    #[zbus(property)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn icon_theme_path(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> MenuNode {
        MenuNode {
            id: 7,
            label: Some("About".to_owned()),
            enabled: true,
            submenu: false,
            children: Vec::new(),
        }
    }

    #[test]
    fn layout_signature_matches_the_dbusmenu_spec() {
        assert_eq!(Layout::SIGNATURE.to_string(), "(ia{sv}av)");
    }

    #[test]
    fn props_carry_label_enabled_visible_and_submenu_marker() {
        let props = node_props(&node(), &[]);
        assert!(matches!(props.get("label"), Some(Value::Str(label)) if label.as_str() == "About"));
        assert_eq!(props.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(props.get("visible"), Some(&Value::Bool(true)));
        assert!(!props.contains_key("children-display"));
        let mut submenu = node();
        submenu.submenu = true;
        let props = node_props(&submenu, &[]);
        assert!(
            matches!(props.get("children-display"), Some(Value::Str(v)) if v.as_str() == "submenu")
        );
    }

    #[test]
    fn separators_advertise_their_type_instead_of_a_label() {
        let mut separator = node();
        separator.label = None;
        let props = node_props(&separator, &[]);
        assert!(!props.contains_key("label"));
        assert!(matches!(props.get("type"), Some(Value::Str(v)) if v.as_str() == "separator"));
    }

    #[test]
    fn property_filter_keeps_only_requested_names() {
        let props = node_props(&node(), &["label".to_owned()]);
        assert_eq!(props.len(), 1);
        assert!(props.contains_key("label"));
    }
}
