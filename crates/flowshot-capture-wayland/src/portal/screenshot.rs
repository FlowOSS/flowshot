//! The `org.freedesktop.portal.Screenshot` capture chain and backend impl.
//!
//! # Chain (one-shot)
//!
//! `Screenshot::request().interactive(flag).modal(false).send()` -> response
//! status 0 -> `file:` URI -> decode -> delete the temp file -> detect the
//! composite pixel space against the enumerated layout -> per-output crops
//! inverse-remapped into the shared [`Frame`] contract. Status 1/2
//! (dismissed/other) map to [`PortalErrorKind::Denied`]; the request runs
//! under the 15s portal budget (draft F27 parity).
//!
//! # Interactive variant (ladder rung 5)
//!
//! `interactive: true` is the GNOME-fallback UX: the compositor's picker
//! lets the user choose a region and the picked image goes straight to the
//! editor (the overlay is skipped). The picked image is NOT a layout
//! composite, so it is returned as a single [`OutputRef::Composite`] frame
//! and never cropped per output.
//!
//! # Cursor
//!
//! The portal decides cursor inclusion (`XDPH` shells out to `grim`, which
//! excludes the cursor); [`CaptureOpts::paint_cursor`] cannot be honored on
//! this path - a documented degradation, never a failure.

use async_trait::async_trait;
use bytes::BytesMut;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, FrameBuffer,
    FrameFormat, OutputRef, PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo, Transform};

use super::composite::{composite_layout, crop_frames, verify_pixel_space};
use super::error::{PortalErrorKind, PortalScreenshotError};
use super::run::{
    PORTAL_TIMEOUT, Selection, build_runtime, classify, collect_outputs, select_outputs,
    with_deadline,
};
use crate::stitch::CapturedOutputs;
use crate::worker::spawn_worker;

/// What one portal screenshot run produced.
pub(crate) enum PortalCapture {
    /// A non-interactive full-layout composite, cropped per output.
    Outputs(CapturedOutputs),
    /// An interactive picker result: one image, already the selection.
    Picked(Frame),
}

impl PortalCapture {
    /// The frames of this capture (a picked image is a single frame).
    pub(crate) fn into_frames(self) -> Vec<Frame> {
        match self {
            Self::Outputs(captured) => captured.frames,
            Self::Picked(frame) => vec![frame],
        }
    }
}

/// Runs one complete portal screenshot capture on the worker thread.
///
/// The portal always captures the FULL layout (there is no per-output
/// request), so `selection` validates up front (a typed `OutputNotFound`
/// before any portal call) and restricts the cropped result afterwards.
/// The interactive picker ignores the selection entirely: its image is the
/// user's choice, not a layout region.
///
/// # Errors
///
/// Any [`PortalScreenshotError`] of the chain: connect/collection failures,
/// portal denials, timeouts, decode failures, and pixel-space mismatches.
pub(crate) fn screenshot_run(
    interactive: bool,
    selection: &Selection,
) -> Result<PortalCapture, PortalScreenshotError> {
    let outputs = collect_outputs::<PortalScreenshotError>()?;
    let selected = select_outputs(outputs.clone(), selection)?;
    let runtime = build_runtime::<PortalScreenshotError>()?;
    let path = runtime.block_on(with_deadline(PORTAL_TIMEOUT, screenshot_path(interactive)))?;
    let rgba = read_screenshot_file(&path)?;

    if interactive {
        tracing::debug!(
            width = rgba.width,
            height = rgba.height,
            "interactive portal picker returned a region image"
        );
        return Ok(PortalCapture::Picked(rgba.into_frame()));
    }

    let layout = composite_layout(&outputs)?;
    let dims = (rgba.width, rgba.height);
    verify_pixel_space(dims, &outputs, &layout)?;
    tracing::debug!(
        width = dims.0,
        height = dims.1,
        "portal screenshot composite detected in physical pixel space"
    );
    let frames = crop_frames(&rgba.data, dims, &outputs, &layout)?;
    let captured = restrict_to(CapturedOutputs { outputs, frames }, &selected);
    Ok(PortalCapture::Outputs(captured))
}

/// Keeps only the outputs (and their frames) the selection validated.
fn restrict_to(captured: CapturedOutputs, selected: &[OutputInfo]) -> CapturedOutputs {
    let pairs: Vec<(OutputInfo, Frame)> = captured
        .outputs
        .into_iter()
        .zip(captured.frames)
        .filter(|(output, _)| {
            selected
                .iter()
                .any(|keep| keep.connector == output.connector)
        })
        .collect();
    let (outputs, frames) = pairs.into_iter().unzip();
    CapturedOutputs { outputs, frames }
}

/// The decoded screenshot plus its dimensions.
struct DecodedScreenshot {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl DecodedScreenshot {
    fn into_frame(self) -> Frame {
        let stride = self.width.saturating_mul(4);
        Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(self.data.as_slice()),
                width: self.width,
                height: self.height,
                stride,
                format: FrameFormat::Rgba8888,
            },
            output: OutputRef::Composite,
            scale: 1.0,
            transform: Transform::Normal,
        }
    }
}

/// Requests the screenshot from the portal and resolves its `file:` URI.
///
/// ashpd 0.13 returns its own minimal [`Uri`](ashpd::Uri) (no `url`
/// dependency), so the `file:` decode runs through `url::Url`: parse, then
/// `to_file_path` (percent-decoding + scheme check). A URI that fails
/// either step is the typed [`PortalErrorKind::FileUri`].
async fn screenshot_path(interactive: bool) -> Result<std::path::PathBuf, PortalScreenshotError> {
    let request = classify(
        ashpd::desktop::screenshot::Screenshot::request()
            .interactive(interactive)
            .modal(false)
            .send()
            .await,
    )?;
    let response = classify(request.response())?;
    let uri = response.uri();
    let file_uri_error = || PortalErrorKind::FileUri {
        uri: uri.to_string(),
    };
    url::Url::parse(uri.as_str())
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| file_uri_error().into())
}

/// Decodes the portal's temp file to RGBA and deletes it (the portal writes
/// a fresh temp file per request; leaving it behind leaks disk on every
/// capture). A decode failure still deletes the file.
///
/// # Errors
///
/// [`PortalErrorKind::Decode`] when the file is not a readable image and
/// [`PortalErrorKind::PermissionDenied`] when the decoded image is the
/// compositor's denial black frame (reachable through the `grim`-based
/// `XDPH` path on `Hyprland` enforce-permissions).
fn read_screenshot_file(
    path: &std::path::Path,
) -> Result<DecodedScreenshot, PortalScreenshotError> {
    let decoded = image::open(path).map(|image| image.to_rgba8());
    if let Err(error) = std::fs::remove_file(path) {
        tracing::warn!(
            %error,
            path = %path.display(),
            "could not delete the portal screenshot temp file"
        );
    }
    let rgba = decoded.map_err(|source| PortalErrorKind::Decode { source })?;
    let (width, height) = rgba.dimensions();
    let data = rgba.into_raw();
    let buffer = FrameBuffer {
        data: BytesMut::from(data.as_slice()),
        width,
        height,
        stride: width.saturating_mul(4),
        format: FrameFormat::Rgba8888,
    };
    if crate::denial::is_denial_frame(&buffer) {
        return Err(PortalErrorKind::PermissionDenied.into());
    }
    Ok(DecodedScreenshot {
        data,
        width,
        height,
    })
}

#[async_trait]
impl CaptureBackend for super::PortalScreenshotBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PortalScreenshot
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        spawn_worker("flowshot-portal-screenshot-outputs", || {
            collect_outputs::<PortalScreenshotError>()
        })
        .await
    }

    async fn capture_outputs(&self, _opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let interactive = self.interactive;
        let captured = spawn_worker("flowshot-portal-screenshot-capture", move || {
            screenshot_run(interactive, &Selection::All)
        })
        .await?;
        Ok(captured.into_frames())
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        let interactive = self.interactive;
        let captured = spawn_worker("flowshot-portal-screenshot-region", move || {
            screenshot_run(interactive, &Selection::All)
        })
        .await?;
        match captured {
            // The interactive picker image IS the region result (rung 5:
            // straight to the editor, the requested region is advisory).
            PortalCapture::Picked(frame) => Ok(frame),
            PortalCapture::Outputs(captured) => {
                captured.stitch(BackendKind::PortalScreenshot, region)
            }
        }
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // The Screenshot portal has no cursor-observation channel; cursor
        // inclusion is the portal frontend's decision. None is the
        // contract's degradation, never a capture failure.
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        // Probe = one real non-interactive request: portals decide per
        // request (there is no pre-capture permission API), so the only
        // honest probe is a capture. Denial (status 1/2 or a denial frame)
        // maps to Denied; any other failure reports the safe side.
        match spawn_worker("flowshot-portal-screenshot-permission", || {
            screenshot_run(false, &Selection::All)
        })
        .await
        {
            Ok(_captured) => PermissionResult::Granted,
            Err(CaptureError::Backend { source, .. })
                if source
                    .downcast_ref::<PortalScreenshotError>()
                    .is_some_and(PortalScreenshotError::is_denied) =>
            {
                PermissionResult::Denied
            }
            Err(error) => {
                tracing::warn!(%error, "portal permission probe failed; reporting denied");
                PermissionResult::Denied
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use bytes::BytesMut;
    use flowshot_capture::{FrameBuffer, FrameFormat};
    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output(connector: &str, x: f64) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(Logical(x), Logical(0.0), Logical(4.0), Logical(3.0)),
            PhysicalSize::new(PhysicalPx(4), PhysicalPx(3)),
            1.0,
            Transform::Normal,
        )
        .unwrap()
    }

    fn frame(connector: &str) -> Frame {
        Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(&[7u8; 48][..]),
                width: 4,
                height: 3,
                stride: 16,
                format: FrameFormat::Rgba8888,
            },
            output: OutputRef::Connector(connector.to_owned()),
            scale: 1.0,
            transform: Transform::Normal,
        }
    }

    #[test]
    fn named_selection_restricts_the_full_layout_crop() {
        // Given a full-layout capture of two outputs,
        let captured = CapturedOutputs {
            outputs: vec![output("HDMI-A-1", 0.0), output("DP-3", 4.0)],
            frames: vec![frame("HDMI-A-1"), frame("DP-3")],
        };
        // When restricting to the DP-3 selection,
        let restricted = restrict_to(captured, &[output("DP-3", 4.0)]);
        // Then only DP-3's output/frame pair survives, still paired.
        assert_eq!(restricted.outputs.len(), 1);
        assert_eq!(restricted.outputs[0].connector, "DP-3");
        assert_eq!(
            restricted.frames[0].output,
            OutputRef::Connector("DP-3".to_owned())
        );
    }

    #[test]
    fn all_selection_keeps_every_pair() {
        let outputs = vec![output("HDMI-A-1", 0.0), output("DP-3", 4.0)];
        let captured = CapturedOutputs {
            frames: vec![frame("HDMI-A-1"), frame("DP-3")],
            outputs: outputs.clone(),
        };
        let restricted = restrict_to(captured, &outputs);
        assert_eq!(restricted.outputs, outputs);
        assert_eq!(restricted.frames.len(), 2);
    }
}
