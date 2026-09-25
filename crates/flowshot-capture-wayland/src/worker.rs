//! The async bridge for one-shot blocking capture workers.
//!
//! Every capture backend runs its blocking Wayland chain on a short-lived
//! worker thread holding a one-shot connection, and bridges the result into a
//! runtime-agnostic future. This module owns that bridge so the
//! `ext-image-copy-capture-v1` and `wlr-screencopy-v1` backends share one
//! implementation; the concrete backend error type is a parameter, so failures
//! are tagged with the right [`BackendKind`](flowshot_capture::BackendKind).

use std::future::Future;

use flowshot_capture::CaptureError;
use futures::FutureExt;

use crate::error::BackendError;

/// Runs one blocking capture operation on a dedicated worker thread and
/// bridges the result into a non-blocking future.
///
/// The worker owns the one-shot connection; if the returned future is dropped
/// (caller cancelled), the worker still runs to completion and its connection
/// teardown releases every compositor-side resource - the result is simply
/// discarded. A worker that ends without sending (a panic) surfaces as a typed
/// [`CaptureError::Backend`] rather than a hang.
pub(crate) fn spawn_worker<T, E, F>(
    name: &str,
    work: F,
) -> impl Future<Output = Result<T, CaptureError>> + Send
where
    T: Send + 'static,
    E: BackendError,
    CaptureError: From<E>,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    let (sender, receiver) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            if sender.send(work()).is_err() {
                tracing::debug!("capture result discarded: the requesting future was cancelled");
            }
        });
    match spawned {
        Ok(_worker) => futures::future::Either::Left(receiver.map(|received| {
            received
                .map_err(|_| CaptureError::Backend {
                    backend: E::KIND,
                    source: E::internal("the capture worker ended without a result (panic?)")
                        .into(),
                })?
                .map_err(CaptureError::from)
        })),
        Err(error) => {
            futures::future::Either::Right(futures::future::ready(Err(CaptureError::Backend {
                backend: E::KIND,
                source: E::from(error).into(),
            })))
        }
    }
}
