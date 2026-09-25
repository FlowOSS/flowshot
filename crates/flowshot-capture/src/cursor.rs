//! The cursor event stream contract.
//!
//! Backends that can observe the compositor's cursor session bridge its
//! enter/leave/position/hotspot events into a [`CursorStream`]. Consumers must
//! treat a missing stream as a graceful degradation (no magnifier tracking),
//! never as a capture failure.

use std::pin::Pin;

use flowshot_core::geometry::{LogicalPoint, PhysicalPoint};
use futures::Stream;

/// A backend's stream of cursor events.
///
/// Boxed and pinned so the trait stays object-safe and implementations can
/// choose their channel (tokio mpsc, calloop channel, synthetic iterator).
pub type CursorStream = Pin<Box<dyn Stream<Item = CursorEvent> + Send>>;

/// One cursor observation from a backend's cursor session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorEvent {
    /// The cursor entered the captured source.
    Entered,
    /// The cursor left the captured source.
    Left,
    /// The cursor moved to a position in global logical layout space.
    Moved {
        /// The new cursor position.
        position: LogicalPoint,
    },
    /// The cursor image's hotspot offset changed, in physical pixels.
    Hotspot {
        /// The hotspot offset inside the cursor image.
        offset: PhysicalPoint,
    },
}
