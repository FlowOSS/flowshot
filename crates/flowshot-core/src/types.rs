//! Core types for `FlowShot`.

/// A screenshot result.
#[derive(Debug, Clone)]
pub struct Screenshot {
    /// The image data.
    pub data: Vec<u8>,
    /// The dimensions of the image.
    pub width: u32,
    /// The height of the image.
    pub height: u32,
}
