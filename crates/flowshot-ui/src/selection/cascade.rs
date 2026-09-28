//! The Esc cascade (draft F27).
//!
//! Flameshot `deleteToolWidgetOrClose` (`capturewidget.cpp` L561-589) walks
//! SIX stages in an exact order, popping the topmost occupied one per Esc
//! press; only an empty cascade closes the overlay:
//!
//! 1. deselect tool (uncheck the active tool button)
//! 2. deselect object (clear the panel's active layer)
//! 3. hide panel
//! 4. delete tool widget
//! 5. hide picker (the color wheel)
//! 6. close
//!
//! Stages 1-5 belong to the editor and chrome layers;
//! [`CascadeState`] is their seam - those layers set and clear the
//! flags, and the cascade order below is the final contract. Every
//! step logs its stable stage token (`flowshot_ui::selection`, field
//! `stage`), so the QA assertion is on structured tokens, never prose.

/// Which overlay sub-UIs are currently occupying the Esc cascade.
///
/// Bit-encoded (one flag per stage) to keep the state a single `u8`; the
/// stage order is the [`EscStep`] declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CascadeState(u8);

impl CascadeState {
    const TOOL: u8 = 0b0000_0001;
    const OBJECT: u8 = 0b0000_0010;
    const PANEL: u8 = 0b0000_0100;
    const WIDGET: u8 = 0b0000_1000;
    const PICKER: u8 = 0b0001_0000;

    /// The empty cascade (Esc closes immediately).
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether a tool is checked (an editor seam).
    #[must_use]
    pub const fn tool_checked(&self) -> bool {
        self.0 & Self::TOOL != 0
    }

    /// Whether an object is selected (an editor seam).
    #[must_use]
    pub const fn object_selected(&self) -> bool {
        self.0 & Self::OBJECT != 0
    }

    /// Whether the side panel is visible (a chrome seam).
    #[must_use]
    pub const fn panel_visible(&self) -> bool {
        self.0 & Self::PANEL != 0
    }

    /// Whether a tool widget exists (an editor seam).
    #[must_use]
    pub const fn tool_widget_present(&self) -> bool {
        self.0 & Self::WIDGET != 0
    }

    /// Whether the color picker is visible (a chrome seam).
    #[must_use]
    pub const fn picker_visible(&self) -> bool {
        self.0 & Self::PICKER != 0
    }

    /// Sets or clears the tool-checked stage.
    pub fn set_tool_checked(&mut self, on: bool) {
        self.set(Self::TOOL, on);
    }

    /// Sets or clears the object-selected stage.
    pub fn set_object_selected(&mut self, on: bool) {
        self.set(Self::OBJECT, on);
    }

    /// Sets or clears the panel-visible stage.
    pub fn set_panel_visible(&mut self, on: bool) {
        self.set(Self::PANEL, on);
    }

    /// Sets or clears the tool-widget stage.
    pub fn set_tool_widget_present(&mut self, on: bool) {
        self.set(Self::WIDGET, on);
    }

    /// Sets or clears the picker-visible stage.
    pub fn set_picker_visible(&mut self, on: bool) {
        self.set(Self::PICKER, on);
    }

    fn set(&mut self, bit: u8, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

/// One Esc cascade step, in the exact spec order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscStep {
    /// Uncheck the active tool.
    DeselectTool,
    /// Clear the selected object.
    DeselectObject,
    /// Hide the side panel.
    HidePanel,
    /// Delete the active tool widget.
    DeleteToolWidget,
    /// Hide the color picker.
    HidePicker,
    /// Close the overlay.
    Close,
}

impl EscStep {
    /// The stable tracing token for this step (QA asserts the sequence on
    /// these tokens).
    #[must_use]
    pub const fn stage_token(self) -> &'static str {
        match self {
            Self::DeselectTool => "deselect-tool",
            Self::DeselectObject => "deselect-object",
            Self::HidePanel => "hide-panel",
            Self::DeleteToolWidget => "delete-tool-widget",
            Self::HidePicker => "hide-picker",
            Self::Close => "close",
        }
    }
}

/// Walks the cascade: returns the topmost occupied stage's step and clears
/// that stage; [`EscStep::Close`] when nothing is occupied. Logs the step's
/// stage token.
pub(super) fn advance_cascade(state: &mut CascadeState) -> EscStep {
    let step = if state.tool_checked() {
        state.set_tool_checked(false);
        EscStep::DeselectTool
    } else if state.object_selected() {
        state.set_object_selected(false);
        EscStep::DeselectObject
    } else if state.panel_visible() {
        state.set_panel_visible(false);
        EscStep::HidePanel
    } else if state.tool_widget_present() {
        state.set_tool_widget_present(false);
        EscStep::DeleteToolWidget
    } else if state.picker_visible() {
        state.set_picker_visible(false);
        EscStep::HidePicker
    } else {
        EscStep::Close
    };
    tracing::info!(
        target: "flowshot_ui::selection",
        stage = step.stage_token(),
        "esc cascade"
    );
    step
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn empty_cascade_closes() {
        let mut state = CascadeState::empty();
        assert_eq!(advance_cascade(&mut state), EscStep::Close);
    }

    #[test]
    fn cascade_pops_stages_in_exact_spec_order() {
        let mut state = CascadeState::empty();
        state.set_tool_checked(true);
        state.set_object_selected(true);
        state.set_panel_visible(true);
        state.set_tool_widget_present(true);
        state.set_picker_visible(true);
        let order = [
            EscStep::DeselectTool,
            EscStep::DeselectObject,
            EscStep::HidePanel,
            EscStep::DeleteToolWidget,
            EscStep::HidePicker,
            EscStep::Close,
        ];
        for expected in order {
            assert_eq!(advance_cascade(&mut state), expected);
        }
        // Every stage cleared; further Esc stays Close.
        assert_eq!(state, CascadeState::empty());
        assert_eq!(advance_cascade(&mut state), EscStep::Close);
    }

    #[test]
    fn cascade_skips_unoccupied_middle_stages() {
        let mut state = CascadeState::empty();
        state.set_panel_visible(true);
        state.set_picker_visible(true);
        assert_eq!(advance_cascade(&mut state), EscStep::HidePanel);
        assert_eq!(advance_cascade(&mut state), EscStep::HidePicker);
        assert_eq!(advance_cascade(&mut state), EscStep::Close);
    }

    #[test]
    fn stage_tokens_are_stable() {
        assert_eq!(EscStep::DeselectTool.stage_token(), "deselect-tool");
        assert_eq!(EscStep::DeselectObject.stage_token(), "deselect-object");
        assert_eq!(EscStep::HidePanel.stage_token(), "hide-panel");
        assert_eq!(
            EscStep::DeleteToolWidget.stage_token(),
            "delete-tool-widget"
        );
        assert_eq!(EscStep::HidePicker.stage_token(), "hide-picker");
        assert_eq!(EscStep::Close.stage_token(), "close");
    }
}
