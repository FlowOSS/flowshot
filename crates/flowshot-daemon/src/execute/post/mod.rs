//! The post-capture pipeline: one completed image ->
//! effective action set -> clipboard/save/upload/notify/pin, the stdout
//! modes (`--raw`, `--print-geometry`), region-memory persistence, and the
//! daemon persistence-reason updates.

use std::path::Path;
use std::sync::Arc;

use std::io::Write;

use flowshot_actions::ExportError;
use flowshot_actions::clipboard::{Action, Clipboard, PostCapture, run_post_capture};
use flowshot_actions::export::{format_geometry, write_raw_png};
use flowshot_actions::upload::{Imgur, UploadHistory};
use flowshot_core::config::{SaveAction, SaveConfig};
use flowshot_core::geometry::LogicalRect;
use flowshot_ui::{Completion, CompletionKind, ExportedImage};
use image::DynamicImage;

mod sinks;

use super::pin;
use super::{ExecCtx, ExecOutcome, ExecuteError};

use crate::notify::{DesktopNotifier, Notifier};
use crate::request::CaptureRequest;
use sinks::NotifyBridge;
pub use sinks::PicturesDirDialog;

/// `--raw` and `--print-geometry` are exclusive stdout modes (one owns the
/// stream); the CLI rejects the combination before any capture work.
///
/// # Errors
///
/// [`ExecuteError::Usage`] when both stdout modes are requested.
pub fn validate_stdout_modes(request: &CaptureRequest) -> Result<(), ExecuteError> {
    if request.raw && request.print_geometry {
        return Err(ExecuteError::Usage(
            crate::strings::STDOUT_MODES_CONFLICT.to_owned(),
        ));
    }
    Ok(())
}

/// Runs the post-capture stage for one completion.
///
/// # Errors
///
/// [`ExecuteError`] for stdout/IO failures; per-action failures are
/// best-effort inside the report.
pub async fn run_post(
    completion: Completion,
    request: &CaptureRequest,
    config: &flowshot_core::Config,
    config_path: Option<&Path>,
    ctx: &ExecCtx,
) -> Result<ExecOutcome, ExecuteError> {
    let selection = completion.selection;
    if let Some(state) = ctx.state.as_ref() {
        state.set_last_capture(completion.image.clone());
    }
    let image = to_dynamic_image(&completion.image)?;
    if request.raw {
        let mut stdout = std::io::stdout().lock();
        write_raw_png(&image, &mut stdout)?;
        stdout.flush()?;
        persist_region(config, config_path, selection);
        return Ok(ExecOutcome::Done(
            flowshot_actions::clipboard::PostCaptureReport::default(),
        ));
    }
    if request.print_geometry {
        print_geometry(selection);
    }
    let actions = effective_actions(completion.kind, request, &config.save.actions);
    let pin_requested = actions.contains(&Action::Pin);
    let pipeline_actions: Vec<Action> = actions
        .into_iter()
        .filter(|action| *action != Action::Pin)
        .collect();
    let save_config = save_config_override(&config.save, request);
    let clipboard = Clipboard::wayland();
    let dialog = PicturesDirDialog;
    let notifier = ctx
        .notifier
        .clone()
        .unwrap_or_else(|| Arc::new(DesktopNotifier::portal()) as Arc<dyn Notifier>);
    let notify = NotifyBridge::new(notifier);
    let uploader = uploader_for(
        &pipeline_actions,
        &config.upload.client_id,
        ctx.upload_base_url.as_deref(),
    );
    let history = upload_history(config, config_path);
    let report = run_post_capture(
        &pipeline_actions,
        &PostCapture {
            image: &image,
            save_config: &save_config,
            notifications_enabled: config.daemon.notifications,
            clipboard: &clipboard,
            dialog: &dialog,
            notify: &notify,
            uploader: uploader.as_deref(),
            upload_history: history.as_ref(),
            copy_upload_url: config.upload.copy_url,
        },
    )
    .await;
    if report
        .outcomes
        .contains(&flowshot_actions::clipboard::ActionOutcome::Copied)
        && let Some(state) = ctx.state.as_ref()
    {
        // The daemon process now serves the data-control offer (a
        // persistence reason); release detection is the known wl-clipboard
        // gap (observed 2026-09-26) - conservative direction.
        state.set_clipboard_offer_held(true);
    }
    if pin_requested {
        pin::spawn_pin(&completion.image, config, ctx).await?;
    }
    persist_region(config, config_path, selection);
    Ok(ExecOutcome::Done(report))
}

/// The gesture/flag/config merge into one ordered effective action set
/// (the `[save].actions` model: flags ADD per-invocation,
/// toolbar gestures RESTRICT to themselves).
fn effective_actions(
    kind: CompletionKind,
    request: &CaptureRequest,
    configured: &[SaveAction],
) -> Vec<Action> {
    let restricted: Option<Vec<Action>> = match &kind {
        CompletionKind::Accept => None,
        CompletionKind::Copy => Some(vec![Action::Copy]),
        CompletionKind::Save => Some(vec![Action::Save]),
        CompletionKind::Pin => Some(vec![Action::Pin]),
        CompletionKind::Upload => Some(vec![Action::Upload]),
        CompletionKind::OpenWith => Some(vec![Action::Save, Action::OpenWith]),
    };
    if let Some(restricted) = restricted {
        return restricted;
    }
    let mut actions: Vec<Action> = configured.iter().copied().map(Action::from).collect();
    for flag in [
        (request.copy, Action::Copy),
        (request.output.is_some(), Action::Save),
        (request.pin, Action::Pin),
        (request.upload, Action::Upload),
    ] {
        if flag.0 && !actions.contains(&flag.1) {
            actions.push(flag.1);
        }
    }
    actions
}

fn save_config_override(save: &SaveConfig, request: &CaptureRequest) -> SaveConfig {
    let mut config = save.clone();
    if let Some(output) = &request.output {
        config.path.clone_from(output);
        config.path_fixed = true;
    }
    config
}

fn to_dynamic_image(image: &ExportedImage) -> Result<DynamicImage, ExecuteError> {
    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone()).ok_or(
        ExportError::ImageEncode("exported image dimensions disagree with its pixels".to_owned()),
    )?;
    Ok(DynamicImage::ImageRgba8(rgba))
}

fn uploader_for(
    actions: &[Action],
    client_id: &str,
    base_url: Option<&str>,
) -> Option<Box<dyn flowshot_actions::upload::Uploader>> {
    if !actions.contains(&Action::Upload) {
        return None;
    }
    if client_id.trim().is_empty() {
        tracing::warn!("upload requested with an unconfigured [upload].client_id; deferred");
        return None;
    }
    Some(match base_url {
        Some(base_url) => Box::new(Imgur::with_base_url(client_id, base_url)),
        None => Box::new(Imgur::new(client_id)),
    })
}

fn upload_history(
    config: &flowshot_core::Config,
    config_path: Option<&Path>,
) -> Option<UploadHistory> {
    let path = match config_path {
        Some(path) => path.parent()?.join("upload-history.json"),
        None => crate::paths::default_config_path()
            .ok()?
            .parent()?
            .join("upload-history.json"),
    };
    match UploadHistory::open(path, config.upload.history_max) {
        Ok(history) => Some(history),
        Err(error) => {
            tracing::warn!(%error, "upload history unavailable; recording skipped");
            None
        }
    }
}

/// Region memory for the direct path (the overlay path persists through
/// the ui `RegionSink`): every completed capture writes
/// `[capture].last_region` when `save_last_region` is on.
fn persist_region(
    config: &flowshot_core::Config,
    config_path: Option<&Path>,
    selection: LogicalRect,
) {
    if !config.capture.save_last_region {
        return;
    }
    let Some(path) = config_path else {
        return;
    };
    let region = flowshot_core::config::Region {
        x: round(selection.x.0),
        y: round(selection.y.0),
        width: u32::try_from(round(selection.width.0)).unwrap_or(0).max(1),
        height: u32::try_from(round(selection.height.0)).unwrap_or(0).max(1),
    };
    let mut updated = config.clone();
    updated.capture.last_region = Some(region);
    if let Err(error) = updated.save(path) {
        tracing::warn!(%error, "last-region persistence failed");
    }
}

fn round(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped into the i32 range above"
    )]
    let edge = clamped as i32;
    edge
}

#[expect(
    clippy::print_stdout,
    reason = "--print-geometry writes its artifact to stdout by contract"
)]
fn print_geometry(selection: LogicalRect) {
    let (Ok(width), Ok(height)) = (
        u32::try_from(round(selection.width.0)),
        u32::try_from(round(selection.height.0)),
    ) else {
        tracing::error!("selection geometry exceeds the printed-integer range");
        return;
    };
    println!(
        "{}",
        format_geometry(round(selection.x.0), round(selection.y.0), width, height)
    );
}
