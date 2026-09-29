//! Renderer value types: where a frame is rendered ([`RenderTarget`]) and
//! what one frame cost ([`FrameStats`]). Split from [`super::renderer`] so the
//! orchestration file stays under the 250-LOC ceiling; these are pure data
//! with no GPU handles of their own.

use std::time::Duration;

/// Where one frame is rendered: a color attachment view (surface texture or
/// offscreen target) and its extent in physical pixels.
#[derive(Debug)]
pub struct RenderTarget<'a> {
    /// The attachment the MSAA frame resolves into.
    pub view: &'a wgpu::TextureView,
    /// Extent in physical pixels.
    pub width: u32,
    /// Extent in physical pixels.
    pub height: u32,
}

/// Per-frame measurements returned by [`super::renderer::Renderer::render`].
/// `Default` is the zero stats the surface presents for an empty (no-content)
/// frame, where the renderer walk never runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameStats {
    /// Display-list commands consumed.
    pub commands: usize,
    /// Vertices staged across all families.
    pub vertices: usize,
    /// Draw calls encoded.
    pub draws: usize,
    /// CPU time for the display-list walk: lyon tessellation + cosmic-text
    /// shaping + quad expansion (the CPU-heavy half the <8 ms budget guards).
    pub build_time: Duration,
    /// CPU time for build + encode + submit (GPU work overlaps asynchronously).
    pub cpu_time: Duration,
    /// Wall time blocked in `Surface::get_current_texture` acquiring the next
    /// swapchain image (the FIFO present-paced wait; zero for offscreen
    /// renders, which the renderer itself measures). Set by the surface path.
    pub acquire_time: Duration,
}

impl FrameStats {
    /// CPU time for staging upload + command encode + `queue.submit`
    /// (the GPU-facing half: `cpu_time - build_time`).
    #[must_use]
    pub fn encode_time(&self) -> Duration {
        self.cpu_time.saturating_sub(self.build_time)
    }
}
