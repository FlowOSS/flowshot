//! The F27 event-routing priority table as a pure decision function.
//!
//! Draft F27 (Flameshot `capturewidget.cpp` `mousePressEvent` L876-918 @
//! 2d478061): event priority is
//!
//! 1. color-picker visible -> the picker consumes,
//! 2. right-click -> open the picker, EXCEPT during a text edit (the
//!    edit-mode tool consumes it),
//! 3. active tool -> `drawStart`/`drawMove`/`drawEnd` (the move tool is
//!    excluded - `startDrawObjectTool` skips `TYPE_MOVESELECTION`, so it
//!    falls through to the object/selection path),
//! 4. a click outside an active edit widget commits it (and consumes the
//!    press - BORROW-MODIFIED: Flameshot falls through to object-select on
//!    the committing press; one press does ONE thing here),
//! 5. object select at pos - topmost object under the press wins,
//! 6. otherwise the selection engine (region create/move/resize - the
//!    todo-16 engine owns selection geometry; with a draw tool active the
//!    tool wins over the handles, Flameshot parity: `startDrawObjectTool`
//!    runs before any selection handling).
//!
//! The function is PURE (every input is a precomputed flag) so the whole
//! priority table is unit-testable without an event loop - the plan's
//! ">=12 cases incl. right-during-text-edit exception" acceptance.

use winit::event::MouseButton;

/// Everything the press routing decision needs (flags the editor precomputes
/// from its own state + the hit-test).
// The flags are independent routing inputs mirroring the F27 priority
// chain's own conditions; grouping them would obscure the table.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent F27 priority condition"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressRoute {
    /// The color picker (wheel) is visible - cascade stage 5 (todo 26).
    pub picker_visible: bool,
    /// The button that pressed.
    pub button: MouseButton,
    /// A tool edit widget (todo 22 text box) is active.
    pub text_editing: bool,
    /// The press position lies inside the edit widget.
    pub edit_contains: bool,
    /// A draw tool is checked.
    pub tool_active: bool,
    /// The checked tool is the move tool (the Flameshot
    /// `startDrawObjectTool` exclusion - it never opens a draw session).
    pub tool_is_move: bool,
    /// An object lies under the press (topmost hit-test result).
    pub object_at: bool,
}

/// Who consumes a pointer press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressTarget {
    /// P1: the visible picker consumes (todo 26).
    Picker,
    /// P2 exception / P4 inside: the active tool's edit mode consumes.
    ToolEdit,
    /// P2: open the color wheel (shell effect).
    ColorWheel,
    /// P4: commit the edit widget; the press is consumed by the commit.
    CommitEdit,
    /// P3: the active tool opens/continues a draw session.
    ToolDraw,
    /// P5: select the topmost object at the press.
    SelectObject,
    /// P6: the selection engine (region create/move/resize).
    Selection,
}

/// Routes one pointer press through the exact F27 priority chain.
#[must_use]
pub fn route_press(route: &PressRoute) -> PressTarget {
    if route.picker_visible {
        return PressTarget::Picker;
    }
    match route.button {
        MouseButton::Right => {
            if route.text_editing {
                PressTarget::ToolEdit
            } else {
                PressTarget::ColorWheel
            }
        }
        MouseButton::Left => route_left(route),
        // Middle/back/forward have no editor semantics; the selection
        // engine ignores them too (todo-16 contract).
        MouseButton::Middle | MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => {
            PressTarget::Selection
        }
    }
}

fn route_left(route: &PressRoute) -> PressTarget {
    if route.tool_active && !route.tool_is_move {
        // P3 beats P4/P5 - but an active edit widget commits first
        // (Flameshot `startDrawObjectTool` -> `commitCurrentTool`).
        return if route.text_editing {
            if route.edit_contains {
                PressTarget::ToolEdit
            } else {
                PressTarget::CommitEdit
            }
        } else {
            PressTarget::ToolDraw
        };
    }
    if route.text_editing && !route.edit_contains {
        return PressTarget::CommitEdit;
    }
    if route.object_at {
        PressTarget::SelectObject
    } else {
        PressTarget::Selection
    }
}

/// Who consumes a pointer motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTarget {
    /// P1: the visible picker consumes (todo 26).
    Picker,
    /// P3: the open draw session extends (`drawMove`).
    ToolDraw,
    /// P6: the selection engine (region drag; no-op without one).
    Selection,
}

/// Routes one pointer motion: an open draw session owns every move until
/// release (the implicit-grab continuity of the todo-13 router feeds it
/// across monitors); otherwise the selection engine sees it.
#[must_use]
pub fn route_move(picker_visible: bool, drawing: bool) -> MoveTarget {
    if picker_visible {
        MoveTarget::Picker
    } else if drawing {
        MoveTarget::ToolDraw
    } else {
        MoveTarget::Selection
    }
}

/// Who consumes a pointer release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseTarget {
    /// P1: the visible picker consumes (todo 26).
    Picker,
    /// P3: the open draw session ends (`drawEnd` -> scene commit).
    ToolDraw,
    /// P6: the selection engine (region release).
    Selection,
}

/// Routes one pointer release (left-button only; other buttons pass
/// through - the editor opens no session for them).
#[must_use]
pub fn route_release(picker_visible: bool, drawing: bool, left: bool) -> ReleaseTarget {
    if picker_visible {
        ReleaseTarget::Picker
    } else if drawing && left {
        ReleaseTarget::ToolDraw
    } else {
        ReleaseTarget::Selection
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )]

    use super::*;

    const LEFT: MouseButton = MouseButton::Left;
    const RIGHT: MouseButton = MouseButton::Right;

    fn base() -> PressRoute {
        PressRoute {
            picker_visible: false,
            button: LEFT,
            text_editing: false,
            edit_contains: false,
            tool_active: false,
            tool_is_move: false,
            object_at: false,
        }
    }

    /// The acceptance table (plan todo 20: >= 12 cases incl. the
    /// right-during-text-edit exception).
    #[test]
    fn press_priority_table() {
        let cases: Vec<(PressRoute, PressTarget)> = vec![
            // P1: the visible picker beats everything (left AND right).
            (
                PressRoute {
                    picker_visible: true,
                    button: LEFT,
                    tool_active: true,
                    ..base()
                },
                PressTarget::Picker,
            ),
            (
                PressRoute {
                    picker_visible: true,
                    button: RIGHT,
                    ..base()
                },
                PressTarget::Picker,
            ),
            // P2: right-click opens the wheel...
            (
                PressRoute {
                    button: RIGHT,
                    ..base()
                },
                PressTarget::ColorWheel,
            ),
            // ...even with a draw tool active...
            (
                PressRoute {
                    button: RIGHT,
                    tool_active: true,
                    ..base()
                },
                PressTarget::ColorWheel,
            ),
            // ...EXCEPT during a text edit (the F27 exception).
            (
                PressRoute {
                    button: RIGHT,
                    text_editing: true,
                    tool_active: true,
                    ..base()
                },
                PressTarget::ToolEdit,
            ),
            // P3: the active tool draws (beats an object under the press).
            (
                PressRoute {
                    tool_active: true,
                    object_at: true,
                    ..base()
                },
                PressTarget::ToolDraw,
            ),
            // P3 exclusion: the move tool falls through to object select...
            (
                PressRoute {
                    tool_active: true,
                    tool_is_move: true,
                    object_at: true,
                    ..base()
                },
                PressTarget::SelectObject,
            ),
            // ...and to the selection engine when no object is hit.
            (
                PressRoute {
                    tool_active: true,
                    tool_is_move: true,
                    ..base()
                },
                PressTarget::Selection,
            ),
            // P4: with a tool active, a click inside the edit widget goes to
            // the tool; outside commits (and consumes).
            (
                PressRoute {
                    tool_active: true,
                    text_editing: true,
                    edit_contains: true,
                    ..base()
                },
                PressTarget::ToolEdit,
            ),
            (
                PressRoute {
                    tool_active: true,
                    text_editing: true,
                    edit_contains: false,
                    ..base()
                },
                PressTarget::CommitEdit,
            ),
            // P5: no tool -> topmost object at pos selects.
            (
                PressRoute {
                    object_at: true,
                    ..base()
                },
                PressTarget::SelectObject,
            ),
            // P6: nothing else -> the selection engine.
            (base(), PressTarget::Selection),
            // Non-left/right buttons pass through.
            (
                PressRoute {
                    button: MouseButton::Middle,
                    tool_active: true,
                    ..base()
                },
                PressTarget::Selection,
            ),
        ];
        for (route, expected) in cases {
            assert_eq!(route_press(&route), expected, "route {route:?}");
        }
    }

    #[test]
    fn move_and_release_follow_the_open_session() {
        assert_eq!(route_move(false, false), MoveTarget::Selection);
        assert_eq!(route_move(false, true), MoveTarget::ToolDraw);
        assert_eq!(route_move(true, true), MoveTarget::Picker);
        assert_eq!(route_release(false, true, true), ReleaseTarget::ToolDraw);
        // A right release never ends a draw session.
        assert_eq!(route_release(false, true, false), ReleaseTarget::Selection);
        assert_eq!(route_release(false, false, true), ReleaseTarget::Selection);
        assert_eq!(route_release(true, true, true), ReleaseTarget::Picker);
    }
}
