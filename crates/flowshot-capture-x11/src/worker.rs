//! The async bridge for one-shot blocking capture workers.
//!
//! Every operation of this backend runs its blocking `x11rb` chain on a
//! short-lived worker thread holding a one-shot connection, bridged into a
//! runtime-agnostic future - the `flowshot-capture-wayland` worker pattern,
//! pinned to the single [`X11Error`] family (no generic backend-error
//! parameter: X11 has exactly one error type, tagged
//! [`BackendKind::X11`]).

use std::future::Future;

use flowshot_capture::{BackendKind, CaptureError};
use futures::FutureExt;

use crate::error::X11Error;

/// Runs one blocking X11 operation on a dedicated worker thread and bridges
/// the result into a non-blocking future.
///
/// The worker owns the one-shot connection; if the returned future is dropped
/// (caller cancelled), the worker still runs to completion and the connection
/// teardown releases every server-side resource - the result is simply
/// discarded. A worker that ends without sending (a panic) surfaces as a typed
/// [`CaptureError::Backend`] rather than a hang.
pub(crate) fn spawn_worker<T, F>(
    name: &str,
    work: F,
) -> impl Future<Output = Result<T, CaptureError>> + Send
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, X11Error> + Send + 'static,
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
                    backend: BackendKind::X11,
                    source: X11Error::Internal(
                        "the capture worker ended without a result (panic?)",
                    )
                    .into(),
                })?
                .map_err(CaptureError::from)
        })),
        Err(error) => {
            futures::future::Either::Right(futures::future::ready(Err(CaptureError::Backend {
                backend: BackendKind::X11,
                source: X11Error::from(error).into(),
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use futures::executor::block_on;

    use super::*;

    #[test]
    fn worker_bridges_the_success_value() {
        let result = block_on(spawn_worker("test-ok", || Ok(7u32)));
        assert_eq!(result.unwrap(), 7);
    }

    #[test]
    fn worker_bridges_typed_errors_tagged_x11() {
        let result = block_on(spawn_worker("test-err", || -> Result<(), X11Error> {
            Err(X11Error::MissingExtension { name: "RANDR" })
        }));
        match result.unwrap_err() {
            CaptureError::Backend { backend, source } => {
                assert_eq!(backend, BackendKind::X11);
                assert!(source.to_string().contains("RANDR"), "{source}");
            }
            other => panic!("expected CaptureError::Backend, got {other:?}"),
        }
    }

    #[test]
    fn timeout_errors_keep_their_mapping() {
        let result = block_on(spawn_worker("test-timeout", || -> Result<(), X11Error> {
            Err(X11Error::Timeout {
                timeout: std::time::Duration::from_secs(1),
            })
        }));
        assert!(matches!(
            result.unwrap_err(),
            CaptureError::Timeout {
                backend: BackendKind::X11
            }
        ));
    }
}
