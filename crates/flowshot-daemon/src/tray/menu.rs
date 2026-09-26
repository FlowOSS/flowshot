//! The tray menu model: stable ids, the F12-parity entry list with the
//! live-probed per-monitor submenu, and the pure id -> action dispatch
//! table.
//!
//! Ids are STABLE across refreshes (libdbusmenu caches by id): fixed
//! entries keep their constants, output entries are `SCREEN_BASE + probe
//! index` (probe order = the todo-6 registry order, which is also the
//! `CaptureScreen(n)` index semantics). Hotplug changes WHICH children
//! exist, never the id scheme.

use flowshot_core::geometry::OutputInfo;

use crate::command::DaemonCommand;
use crate::request::CaptureRequest;
use crate::strings;

/// The dbusmenu root id (host-owned; never dispatched).
pub const ROOT_ID: i32 = 0;
/// `Take Screenshot` (interactive region capture).
pub const TAKE_SCREENSHOT_ID: i32 = 1;
/// `Capture Full Screen`.
pub const CAPTURE_FULL_ID: i32 = 2;
/// Per-monitor submenu root (`children-display = submenu`).
pub const SCREEN_SUBMENU_ID: i32 = 3;
/// `Capture Launcher` (todo 37 seam).
pub const LAUNCHER_ID: i32 = 4;
/// Separator below the capture group.
pub const SEPARATOR_A_ID: i32 = 5;
/// `Configure` (todo 36 seam).
pub const CONFIGURE_ID: i32 = 6;
/// `About`.
pub const ABOUT_ID: i32 = 7;
/// Separator above Quit.
pub const SEPARATOR_B_ID: i32 = 8;
/// `Quit` (clean daemon shutdown).
pub const QUIT_ID: i32 = 9;
/// Disabled placeholder shown while the output probe knows no monitor.
pub const NO_OUTPUTS_ID: i32 = 99;
/// Base of the per-output entry ids (`SCREEN_BASE + probe index`).
pub const SCREEN_BASE: i32 = 100;
/// Highest supported output index (bounds the id space below the
/// separator-free zone; 99 monitors).
pub const MAX_OUTPUTS: usize = 99;

/// What a menu activation does (the dispatch-table value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// Forward a command to the daemon's [`CommandSink`](crate::CommandSink).
    Command(DaemonCommand),
    /// The About toast (version body).
    About,
    /// Clean daemon shutdown (the todo-32 lifecycle quit seam).
    Quit,
    /// Separators, submenu roots and placeholders: nothing to dispatch.
    None,
}

/// The pure id -> action table (unit-tested without any bus).
#[must_use]
pub fn action_for(id: i32) -> TrayAction {
    match id {
        TAKE_SCREENSHOT_ID => {
            TrayAction::Command(DaemonCommand::Capture(CaptureRequest::default()))
        }
        CAPTURE_FULL_ID => TrayAction::Command(DaemonCommand::CaptureFull),
        LAUNCHER_ID => TrayAction::Command(DaemonCommand::Launcher),
        CONFIGURE_ID => TrayAction::Command(DaemonCommand::Settings),
        ABOUT_ID => TrayAction::About,
        QUIT_ID => TrayAction::Quit,
        SCREEN_BASE..=i32::MAX => match u32::try_from(id - SCREEN_BASE) {
            Ok(screen) if (screen as usize) < MAX_OUTPUTS => {
                TrayAction::Command(DaemonCommand::CaptureScreen(screen))
            }
            _ => TrayAction::None,
        },
        _ => TrayAction::None,
    }
}

/// One menu entry (host-agnostic model; the wire mapping lives in
/// [`super::dbusmenu`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuNode {
    /// Stable dbusmenu id.
    pub id: i32,
    /// Visible label; `None` renders a separator.
    pub label: Option<String>,
    /// Greyed-out entries never dispatch.
    pub enabled: bool,
    /// Whether the node hosts children (`children-display = submenu`).
    pub submenu: bool,
    /// Child entries (only for `submenu` nodes).
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    fn entry(id: i32, label: &str) -> Self {
        Self {
            id,
            label: Some(label.to_owned()),
            enabled: true,
            submenu: false,
            children: Vec::new(),
        }
    }

    fn separator(id: i32) -> Self {
        Self {
            id,
            label: None,
            enabled: false,
            submenu: false,
            children: Vec::new(),
        }
    }
}

/// Builds the root-level menu for the given live-probed outputs (F12
/// parity: Take Screenshot / Capture Launcher / per-monitor submenu /
/// Configure / About / Quit, plus the per-mode full-screen entry).
#[must_use]
pub fn build_menu(outputs: &[OutputInfo]) -> Vec<MenuNode> {
    let mut children = Vec::new();
    if outputs.is_empty() {
        children.push(MenuNode {
            id: NO_OUTPUTS_ID,
            label: Some(strings::MENU_NO_OUTPUTS.to_owned()),
            enabled: false,
            submenu: false,
            children: Vec::new(),
        });
    } else {
        children.extend(
            outputs
                .iter()
                .enumerate()
                .take(MAX_OUTPUTS)
                .map(|(index, output)| {
                    MenuNode::entry(
                        SCREEN_BASE + i32::try_from(index).unwrap_or(i32::MAX),
                        &output_label(index, output),
                    )
                }),
        );
    }
    vec![
        MenuNode::entry(TAKE_SCREENSHOT_ID, strings::MENU_TAKE_SCREENSHOT),
        MenuNode::entry(CAPTURE_FULL_ID, strings::MENU_CAPTURE_FULL),
        MenuNode {
            id: SCREEN_SUBMENU_ID,
            label: Some(strings::MENU_CAPTURE_SCREEN.to_owned()),
            enabled: true,
            submenu: true,
            children,
        },
        MenuNode::entry(LAUNCHER_ID, strings::MENU_CAPTURE_LAUNCHER),
        MenuNode::separator(SEPARATOR_A_ID),
        MenuNode::entry(CONFIGURE_ID, strings::MENU_CONFIGURE),
        MenuNode::entry(ABOUT_ID, strings::MENU_ABOUT),
        MenuNode::separator(SEPARATOR_B_ID),
        MenuNode::entry(QUIT_ID, strings::MENU_QUIT),
    ]
}

/// `Screen {n}: {name}` - the live probe's name already carries make,
/// model AND connector (`Samsung Electric Company T22B350 (HDMI-A-1)`,
/// QA-verified); the connector is the fallback for a nameless output.
fn output_label(index: usize, output: &OutputInfo) -> String {
    let display = if output.name.is_empty() {
        output.connector.as_str()
    } else {
        output.name.as_str()
    };
    format!("Screen {index}: {display}")
}

/// Finds a node by id anywhere in the tree (`None` for unknown ids and
/// for the virtual root, which is not a node).
#[must_use]
pub fn find_node(nodes: &[MenuNode], id: i32) -> Option<&MenuNode> {
    nodes.iter().find_map(|node| {
        if node.id == id {
            Some(node)
        } else {
            find_node(&node.children, id)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    fn output(connector: &str, name: &str) -> OutputInfo {
        OutputInfo::new(
            connector,
            name,
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(1920.0), Logical(1080.0)),
            PhysicalSize::new(PhysicalPx(1920), PhysicalPx(1080)),
            1.0,
            Transform::Normal,
        )
        .unwrap_or_else(|error| panic!("fixture output must be valid: {error}"))
    }

    #[test]
    fn dispatch_table_maps_every_entry_to_its_command() {
        assert_eq!(
            action_for(TAKE_SCREENSHOT_ID),
            TrayAction::Command(DaemonCommand::Capture(CaptureRequest::default()))
        );
        assert_eq!(
            action_for(CAPTURE_FULL_ID),
            TrayAction::Command(DaemonCommand::CaptureFull)
        );
        assert_eq!(
            action_for(LAUNCHER_ID),
            TrayAction::Command(DaemonCommand::Launcher)
        );
        assert_eq!(
            action_for(CONFIGURE_ID),
            TrayAction::Command(DaemonCommand::Settings)
        );
        assert_eq!(action_for(ABOUT_ID), TrayAction::About);
        assert_eq!(action_for(QUIT_ID), TrayAction::Quit);
    }

    #[test]
    fn screen_entries_map_to_their_probe_index() {
        assert_eq!(
            action_for(SCREEN_BASE),
            TrayAction::Command(DaemonCommand::CaptureScreen(0))
        );
        assert_eq!(
            action_for(SCREEN_BASE + 2),
            TrayAction::Command(DaemonCommand::CaptureScreen(2))
        );
        assert_eq!(
            action_for(SCREEN_BASE + i32::try_from(MAX_OUTPUTS).unwrap_or(i32::MAX)),
            TrayAction::None,
            "the first id past the supported range must not dispatch"
        );
    }

    #[test]
    fn structural_ids_never_dispatch() {
        for id in [
            ROOT_ID,
            SCREEN_SUBMENU_ID,
            SEPARATOR_A_ID,
            SEPARATOR_B_ID,
            NO_OUTPUTS_ID,
            -1,
        ] {
            assert_eq!(action_for(id), TrayAction::None, "id {id}");
        }
    }

    #[test]
    fn menu_carries_the_parity_entries_in_order() {
        let menu = build_menu(&[]);
        let ids: Vec<i32> = menu.iter().map(|node| node.id).collect();
        assert_eq!(
            ids,
            vec![
                TAKE_SCREENSHOT_ID,
                CAPTURE_FULL_ID,
                SCREEN_SUBMENU_ID,
                LAUNCHER_ID,
                SEPARATOR_A_ID,
                CONFIGURE_ID,
                ABOUT_ID,
                SEPARATOR_B_ID,
                QUIT_ID,
            ]
        );
        assert_eq!(menu[0].label.as_deref(), Some("Take Screenshot"));
        assert_eq!(menu[3].label.as_deref(), Some("Capture Launcher"));
        assert_eq!(menu[5].label.as_deref(), Some("Configure"));
        assert_eq!(menu[8].label.as_deref(), Some("Quit"));
        assert!(menu[4].label.is_none(), "separators carry no label");
    }

    #[test]
    fn submenu_children_follow_the_probe_order() {
        let outputs = vec![output("DP-1", "LG 27UK850"), output("HDMI-A-1", "")];
        let menu = build_menu(&outputs);
        let submenu = &menu[2];
        assert!(submenu.submenu);
        let children: Vec<(i32, String)> = submenu
            .children
            .iter()
            .map(|node| (node.id, node.label.clone().unwrap_or_default()))
            .collect();
        assert_eq!(
            children,
            vec![
                (SCREEN_BASE, "Screen 0: LG 27UK850".to_owned()),
                (SCREEN_BASE + 1, "Screen 1: HDMI-A-1".to_owned()),
            ]
        );
    }

    #[test]
    fn empty_probe_shows_the_disabled_placeholder() {
        let menu = build_menu(&[]);
        let submenu = &menu[2];
        assert_eq!(submenu.children.len(), 1);
        assert_eq!(submenu.children[0].id, NO_OUTPUTS_ID);
        assert!(!submenu.children[0].enabled);
        assert_eq!(action_for(NO_OUTPUTS_ID), TrayAction::None);
    }

    #[test]
    fn find_node_reaches_submenu_children_and_rejects_strangers() {
        let menu = build_menu(&[output("DP-1", "Monitor")]);
        assert_eq!(
            find_node(&menu, TAKE_SCREENSHOT_ID).map(|n| n.id),
            Some(TAKE_SCREENSHOT_ID)
        );
        assert_eq!(
            find_node(&menu, SCREEN_BASE).map(|n| n.id),
            Some(SCREEN_BASE)
        );
        assert!(find_node(&menu, 12_345).is_none());
    }
}
