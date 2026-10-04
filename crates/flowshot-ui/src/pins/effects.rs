//! Pin effect application (split from [`super::shell`] for the 250-LOC
//! ceiling; both files extend the same `PinApp` impl).

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;

use super::event::PinEffect;
use super::shell::PinApp;
use super::sink::{PinId, PinSnapshot};
use crate::error::UiError;

impl PinApp {
    pub(super) fn apply(
        &mut self,
        target: &ActiveEventLoop,
        index: usize,
        id: PinId,
        effects: &[PinEffect],
    ) {
        for effect in effects {
            match *effect {
                PinEffect::Redraw => {
                    if let Some(entry) = self.entries.get(index) {
                        entry.window.request_redraw();
                    }
                }
                PinEffect::SetWindowSize { width, height } => {
                    if let Some(entry) = self.entries.get(index) {
                        let size: Option<winit::dpi::Size> =
                            Some(PhysicalSize::new(width, height).into());
                        // min == max == target: the compositor-driven resize
                        // path (see the module header).
                        entry.window.set_min_inner_size(size);
                        entry.window.set_max_inner_size(size);
                    }
                }
                PinEffect::Reupload => {
                    if let Err(error) = self.reupload(index) {
                        tracing::error!(%error, pin = id.raw(), "pin texture re-upload failed");
                    }
                }
                PinEffect::StartDrag => {
                    if let Some(entry) = self.entries.get(index)
                        && let Err(error) = entry.window.drag_window()
                    {
                        // Synthetic injections carry no pointer serial;
                        // a refused drag is logged, never fatal.
                        tracing::debug!(%error, pin = id.raw(), "drag_window refused");
                    }
                }
                PinEffect::Close => self.close(target, index, id),
                PinEffect::Copy => self.dispatch_snapshot(index, id, true),
                PinEffect::Save => self.dispatch_snapshot(index, id, false),
            }
        }
    }

    fn reupload(&mut self, index: usize) -> Result<(), UiError> {
        let Some(gpu) = self.gpu.as_ref() else {
            return Ok(());
        };
        let Some(entry) = self.entries.get_mut(index) else {
            return Ok(());
        };
        let buffer = entry
            .image
            .composed(entry.state.rotation(), entry.state.opacity())?;
        let (width, height) = entry.state.image_size();
        entry.renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            super::TEXTURE_ID,
            &crate::render::RgbaImage {
                width,
                height,
                data: &buffer,
            },
        )
    }

    fn dispatch_snapshot(&self, index: usize, id: PinId, copy: bool) {
        let Some(sink) = self.sink.clone() else {
            tracing::warn!(pin = id.raw(), "no action sink wired; copy/save ignored");
            return;
        };
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let snapshot = match entry.image.rotated(entry.state.rotation()) {
            Ok(rgba) => PinSnapshot {
                id,
                width: entry.state.image_size().0,
                height: entry.state.image_size().1,
                rgba,
            },
            Err(error) => {
                tracing::error!(%error, pin = id.raw(), "pin snapshot failed");
                return;
            }
        };
        let outcome = if copy {
            sink.copy(snapshot)
        } else {
            sink.save(snapshot)
        };
        if let Err(error) = outcome {
            tracing::error!(pin = id.raw(), copy, %error, "pin action failed");
        }
    }

    pub(super) fn close(&mut self, target: &ActiveEventLoop, index: usize, id: PinId) {
        if index >= self.entries.len() || self.entries[index].id != id {
            return;
        }
        let entry = self.entries.remove(index);
        self.window_index.remove(&entry.window.id());
        // Re-index the entries that shifted past the removed slot.
        for slot in self.window_index.values_mut() {
            if *slot > index {
                *slot -= 1;
            }
        }
        if let Some(sink) = &self.sink {
            sink.pin_closed(id);
        }
        tracing::info!(pin = id.raw(), remaining = self.entries.len(), "pin closed");
        drop(entry); // dropping the last Arc<Window> destroys the window
        if self.entries.is_empty() {
            target.exit();
        }
    }
}
