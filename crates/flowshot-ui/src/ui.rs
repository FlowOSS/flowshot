//! UI components.

/// A UI element.
#[derive(Debug)]
pub struct UiElement {
    /// The element's position.
    pub position: (u32, u32),
    /// The element's size.
    pub size: (u32, u32),
}
